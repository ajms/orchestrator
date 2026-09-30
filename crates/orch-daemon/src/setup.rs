use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use orch_core::{PhaseEvent, SessionId};
use orch_holder::SESSION_ENV;

use crate::lifecycle::{Busy, session_env};
use crate::state::Daemon;

const SETUP_LOG: &str = "setup.log";
const SETUP_PID: &str = "setup.pid";
const STREAM_EVERY: Duration = Duration::from_millis(200);
const INTERRUPTED: &str = "Setup was interrupted when the Daemon stopped.";

enum Outcome {
    Succeeded,
    Failed,
}

impl Daemon {
    fn setup_log(&self, id: &SessionId) -> PathBuf {
        self.session_dir(id).join(SETUP_LOG)
    }

    pub(crate) async fn read_setup_log(&self, id: &SessionId) -> Option<String> {
        let log = self.setup_log(id);
        tokio::task::spawn_blocking(move || std::fs::read_to_string(log).ok())
            .await
            .ok()
            .flatten()
    }

    pub(crate) async fn run_setup(self: Arc<Self>, id: SessionId) {
        let _busy = Busy::new(&self);
        let Some((record, repo)) = self.snapshot(&id) else {
            return;
        };
        let script = match self.repo_config(&repo).await {
            Ok(config) => match config.setup_script() {
                Ok(script) => Ok(script.map(String::from)),
                Err(untrusted) => Err(format!(
                    "{untrusted}; approve its Setup script, then retry.\n"
                )),
            },
            Err(err) => Err(format!("{err}\n")),
        };
        let dir = self.session_dir(&id);
        let log = self.setup_log(&id);
        let outcome = match script {
            Ok(None) => write_log(&dir, &log, "").await.map(|()| Outcome::Succeeded),
            Ok(Some(command)) => {
                let env = session_env(&record);
                self.run_script(&id, &dir, &log, &command, env, &record.worktree)
                    .await
            }
            Err(message) => write_log(&dir, &log, &message)
                .await
                .map(|()| Outcome::Failed),
        };
        let outcome = outcome.unwrap_or_else(|err| {
            eprintln!("orch daemon: Setup of {}: {err}", id.as_str());
            Outcome::Failed
        });
        let output = self.read_setup_log(&id).await.unwrap_or_default();
        let event = match outcome {
            Outcome::Succeeded => PhaseEvent::SetupSucceeded,
            Outcome::Failed => PhaseEvent::SetupFailed,
        };
        let updated = self.update(&id, |live| {
            live.setup_output = Some(output);
            live.launching = event == PhaseEvent::SetupSucceeded;
            live.transition(event)
        });
        if event == PhaseEvent::SetupSucceeded && updated.is_ok() {
            self.launch_agent(&id, false).await;
        }
    }

    async fn run_script(
        &self,
        id: &SessionId,
        dir: &Path,
        log: &Path,
        command: &str,
        env: Vec<(String, String)>,
        worktree: &Path,
    ) -> std::io::Result<Outcome> {
        write_log(dir, log, "").await?;
        let output = File::options().append(true).open(log)?;
        let mut child = crate::subprocess::command("sh")
            .arg("-c")
            .arg(command)
            .current_dir(worktree)
            .envs(env)
            .stdin(Stdio::null())
            .stdout(output.try_clone()?)
            .stderr(output)
            .process_group(0)
            .kill_on_drop(false)
            .spawn()?;
        let pid_file = dir.join(SETUP_PID);
        if let Some(pid) = child.id() {
            std::fs::write(&pid_file, pid.to_string())?;
        }
        let mut shown = String::new();
        let status = loop {
            tokio::select! {
                status = child.wait() => break status?,
                () = tokio::time::sleep(STREAM_EVERY) => {
                    let current = self.read_setup_log(id).await.unwrap_or_default();
                    if current != shown {
                        shown.clone_from(&current);
                        let _ = self.update(id, |live| {
                            live.setup_output = Some(current);
                            Ok(())
                        });
                    }
                }
            }
        };
        let _ = std::fs::remove_file(pid_file);
        Ok(match status.success() {
            true => Outcome::Succeeded,
            false => Outcome::Failed,
        })
    }

    pub(crate) async fn stop_setup(&self, id: &SessionId) {
        let pid_file = self.session_dir(id).join(SETUP_PID);
        let session = id.clone();
        let _ =
            tokio::task::spawn_blocking(move || kill_orphaned_script(&pid_file, &session)).await;
    }

    pub(crate) async fn interrupted_setup(&self, id: &SessionId) {
        let dir = self.session_dir(id);
        let session = id.clone();
        let log = self.setup_log(id);
        let output = tokio::task::spawn_blocking(move || {
            kill_orphaned_script(&dir.join(SETUP_PID), &session);
            let mut output = std::fs::read_to_string(&log).unwrap_or_default();
            if !output.is_empty() && !output.ends_with('\n') {
                output.push('\n');
            }
            output.push_str(INTERRUPTED);
            let _ = std::fs::write(&log, &output);
            output
        })
        .await
        .unwrap_or_else(|_| INTERRUPTED.into());
        let _ = self.update(id, |live| {
            live.setup_output = Some(output);
            live.transition(PhaseEvent::SetupFailed)
        });
    }
}

async fn write_log(dir: &Path, log: &Path, content: &str) -> std::io::Result<()> {
    let (dir, log, content) = (dir.to_path_buf(), log.to_path_buf(), content.to_string());
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(log, content)
    })
    .await
    .map_err(std::io::Error::other)?
}

fn kill_orphaned_script(pid_file: &Path, session: &SessionId) {
    let Some(pid) = std::fs::read_to_string(pid_file)
        .ok()
        .and_then(|pid| pid.trim().parse::<i32>().ok())
    else {
        return;
    };
    let marker = format!("{SESSION_ENV}={}", session.as_str());
    let ours = std::fs::read(format!("/proc/{pid}/environ")).is_ok_and(|environ| {
        environ
            .split(|byte| *byte == 0)
            .any(|var| var == marker.as_bytes())
    });
    if ours {
        let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
    }
    let _ = std::fs::remove_file(pid_file);
}
