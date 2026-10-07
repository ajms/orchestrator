use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use orch_core::SessionId;
use orch_holder::HolderClient;
use orch_protocol::{
    AgentStateView as State, CreateSession, GuardChoice, GuardPrompt, Reply, Request, Size,
};
use serde_json::{Value, json};

use crate::common::*;
use crate::{fixtures, template};

pub const STARTUP: Duration = Duration::from_secs(90);
pub const TURN: Duration = Duration::from_secs(300);
pub const SCREEN: Size = Size {
    rows: 40,
    cols: 120,
};
pub const TRUST_ANSWER: &[u8] = b"\r";
pub const PERMISSION_ANSWER: &[u8] = b"\r";
pub const SHIFT_TAB: &[u8] = b"\x1b[Z";
const GATE: &str = "ORCH_REAL_AGY";
const RECORD: &str = "ORCH_REAL_AGY_RECORD";
const CAPTURE_ENV: &str = "ORCH_REAL_AGY_CAPTURE";
const CAPTURE_HOOK: &str = "orch-real-agy-capture";

fn enabled(var: &str) -> bool {
    std::env::var(var).is_ok_and(|value| value == "1")
}

pub fn is_trust_screen(lower: &str) -> bool {
    lower.contains("trust")
}

pub fn isolate<'c>(command: &'c mut Command, home: &Path) -> &'c mut Command {
    command
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env_remove("ORCH_SESSION")
        .env_remove("ORCH_HOLDER_SOCKET")
        .env_remove("XDG_RUNTIME_DIR")
}

fn find_agy() -> PathBuf {
    std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join("agy"))
        .find(|path| path.is_file())
        .expect("ORCH_REAL_AGY=1 needs agy on PATH")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_dir(&entry.path(), &target);
        } else if kind.is_symlink() {
            std::os::unix::fs::symlink(std::fs::read_link(entry.path()).unwrap(), target).unwrap();
        } else if kind.is_file() {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

pub struct Held {
    pub client: HolderClient,
    pids: Vec<i32>,
}

impl Drop for Held {
    fn drop(&mut self) {
        for pid in &self.pids {
            kill(*pid, "-KILL");
        }
    }
}

pub async fn hold(
    mut command: Command,
    runtime: &Path,
    session: &str,
    cwd: &Path,
    agy: &Path,
) -> Held {
    let output = command
        .env("ORCH_RUNTIME_DIR", runtime)
        .args(["hold", "--session", session, "--runtime-dir"])
        .arg(runtime)
        .arg("--cwd")
        .arg(cwd)
        .arg("--")
        .arg(agy)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success(), "orch hold: {output:?}");
    let holder: i32 = String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let socket = orch_holder::socket_path(runtime, &SessionId::parse(session).unwrap());
    let mut client = HolderClient::connect(&socket).await.unwrap();
    let hello = client.attach().await.unwrap();
    let pids = [Some(holder), hello.agent_pid.map(|pid| pid as i32)];
    Held {
        client,
        pids: pids.into_iter().flatten().collect(),
    }
}

pub struct RealAgy {
    pub env: Env,
    pub agy: PathBuf,
    captures: Option<PathBuf>,
}

impl RealAgy {
    pub async fn new() -> Option<Self> {
        if !enabled(GATE) {
            eprintln!("skipped: set {GATE}=1 to run against a real, authenticated agy");
            return None;
        }
        let agy = find_agy();
        let template = template::prepare(&agy).await;
        let env = Env::new();
        copy_dir(&template.join("home"), &env.path("home"));
        copy_dir(&template.join("state"), &env.path("state"));
        env.write_config(&format!(
            "[defaults.agents.antigravity]\nbinary = {:?}\n",
            agy.display().to_string()
        ));
        let captures = enabled(RECORD).then(|| {
            let dir = env.path("captures");
            std::fs::create_dir_all(&dir).unwrap();
            add_capture_hooks(&env.path("home"), &dir);
            dir
        });
        Some(Self { env, agy, captures })
    }

    fn isolate<'c>(&self, command: &'c mut Command) -> &'c mut Command {
        isolate(command, &self.env.path("home"));
        if let Some(dir) = &self.captures {
            command.env(CAPTURE_ENV, dir);
        }
        command
    }

    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut command = Command::new(program);
        self.isolate(&mut command)
            .env("XDG_CONFIG_HOME", self.env.path("config"))
            .env("XDG_STATE_HOME", self.env.path("state"));
        command
    }

    pub async fn start_daemon(&self) -> Daemon {
        let mut command = self.env.daemon_command(Duration::from_secs(3600));
        self.isolate(&mut command);
        self.env.spawn_daemon(&mut command).await
    }

    pub async fn hold(&self, session: &str, cwd: &Path) -> Held {
        let mut command = self.env.orch();
        self.isolate(&mut command);
        hold(command, &self.env.runtime_dir(), session, cwd, &self.agy).await
    }

    pub async fn session(
        &self,
        client: &mut TestClient,
        repo: &Path,
        preset: &str,
        prompt: &str,
    ) -> (SessionId, PaneView) {
        let mut create = CreateSession::new(repo, prompt);
        create.agent = Some("antigravity".into());
        create.preset = Some(preset.into());
        let id = client.create(create).await;
        client
            .until_within(&id, "running", STARTUP, |view| view.agent.is_some())
            .await;
        let pane = self.env.pane(&id, SCREEN).await;
        (id, pane)
    }

    pub fn agy_processes(&self) -> Vec<Vec<String>> {
        self.env
            .processes()
            .into_iter()
            .filter_map(|pid| std::fs::read(format!("/proc/{pid}/cmdline")).ok())
            .map(|cmdline| {
                cmdline
                    .split(|byte| *byte == 0)
                    .filter(|arg| !arg.is_empty())
                    .map(|arg| String::from_utf8_lossy(arg).into_owned())
                    .collect::<Vec<_>>()
            })
            .filter(|argv| {
                argv.first()
                    .is_some_and(|program| Path::new(program).file_name() == Some("agy".as_ref()))
            })
            .collect()
    }

    pub async fn agy_argv(&self, what: &str, wanted: impl Fn(&[String]) -> bool) -> Vec<String> {
        let deadline = Instant::now() + STARTUP;
        loop {
            let running = self.agy_processes();
            if let Some(argv) = running.iter().find(|argv| wanted(argv)) {
                return argv.clone();
            }
            assert!(
                Instant::now() < deadline,
                "no agy ran with {what}; running: {running:?}"
            );
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    pub fn record(&self, client: &TestClient, id: &SessionId) {
        let Some(captures) = &self.captures else {
            return;
        };
        let view = &client.sessions[id];
        let root = view.conversation.clone().expect("a Conversation to record");
        let mut replacements = vec![
            (
                view.worktree.display().to_string(),
                "/home/dev/shop/.orchestrator/worktrees/fix-login".into(),
            ),
            (view.repo.display().to_string(), "/home/dev/shop".into()),
            (
                self.env.path("home").display().to_string(),
                "/home/dev".into(),
            ),
            (self.env.path("").display().to_string(), "/home/dev/".into()),
        ];
        if let Ok(home) = std::env::var("HOME") {
            replacements.push((home, "/home/dev".into()));
        }
        let written = fixtures::record(captures, &root, replacements);
        eprintln!("re-recorded {written:?}");
    }
}

fn add_capture_hooks(home: &Path, dir: &Path) {
    let path = home.join(".gemini/config/hooks.json");
    let mut hooks: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let hook = |event: &str| {
        let mut command = format!("cat > '{}/{event}-'\"$(date +%s%N)\"'.json'", dir.display());
        if event == "PreToolUse" {
            command.push_str(r#"; printf '{"decision":"ask"}'"#);
        }
        json!({ "type": "command", "command": command })
    };
    let tool = |event: &str| json!([{ "matcher": "*", "hooks": [hook(event)] }]);
    hooks[CAPTURE_HOOK] = json!({
        "PreToolUse": tool("PreToolUse"),
        "PostToolUse": tool("PostToolUse"),
        "PreInvocation": [hook("PreInvocation")],
        "PostInvocation": [hook("PostInvocation")],
        "Stop": [hook("Stop")],
    });
    std::fs::write(path, serde_json::to_string_pretty(&hooks).unwrap()).unwrap();
}

pub async fn until_state(client: &mut TestClient, id: &SessionId, state: State, wait: Duration) {
    client
        .until_within(id, &format!("{state:?}"), wait, |view| {
            view.agent == Some(state)
        })
        .await;
}

pub async fn answer_trust(client: &mut TestClient, id: &SessionId, pane: &mut PaneView) {
    until_state(client, id, State::NeedsInput, STARTUP).await;
    pane.wait_for_within("agy's trust screen", STARTUP, |pane| {
        is_trust_screen(&pane.text().to_lowercase())
    })
    .await;
    pane.pane.input(TRUST_ANSWER.to_vec()).await.unwrap();
    client
        .until_within(id, "past the trust screen", STARTUP, |view| {
            view.agent != Some(State::NeedsInput)
        })
        .await;
}

pub async fn deny_guards_until_idle(client: &mut TestClient, id: &SessionId) -> Vec<GuardPrompt> {
    let mut denied = Vec::new();
    loop {
        let view = client
            .until_within(id, "Idle or a Guard prompt", TURN, |view| {
                view.agent == Some(State::Idle) || !view.guard_prompts.is_empty()
            })
            .await;
        let Some(guard) = view.guard_prompts.first().cloned() else {
            return denied;
        };
        let answer = Request::AnswerGuard {
            session: id.clone(),
            guard: guard.id,
            choice: GuardChoice::Deny,
        };
        assert_eq!(client.request(answer).await, Ok(Reply::Done));
        client
            .until_within(id, "the Guard answered", STARTUP, |view| {
                view.guard_prompts
                    .iter()
                    .all(|prompt| prompt.id != guard.id)
            })
            .await;
        denied.push(guard);
    }
}

pub fn run_with_stdin(mut command: Command, input: &str, wait: Duration) -> Output {
    use std::io::Write;
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let deadline = Instant::now() + wait;
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    child.wait_with_output().unwrap()
}

pub fn osc_11_stall(chunks: &[(Duration, Vec<u8>)], end: Duration) -> Option<(Duration, Duration)> {
    const QUERY: &[u8] = b"\x1b]11;?";
    let bytes: Vec<u8> = chunks.iter().flat_map(|(_, chunk)| chunk.clone()).collect();
    let start = bytes
        .windows(QUERY.len())
        .position(|window| window == QUERY)?;
    let after = start + QUERY.len();
    let terminator = if bytes.get(after) == Some(&0x07) {
        1
    } else {
        2
    };
    let query_end = after + terminator;
    let mut offset = 0;
    let mut asked_at = None;
    for (at, chunk) in chunks {
        let chunk_end = offset + chunk.len();
        match asked_at {
            None if query_end <= chunk_end => {
                if query_end < chunk_end {
                    return Some((*at, Duration::ZERO));
                }
                asked_at = Some(*at);
            }
            Some(asked) => return Some((asked, *at - asked)),
            None => {}
        }
        offset = chunk_end;
    }
    asked_at.map(|asked| (asked, end - asked))
}
