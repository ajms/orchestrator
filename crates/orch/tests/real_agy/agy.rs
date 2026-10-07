use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use orch_core::SessionId;
use orch_holder::HolderClient;
use orch_protocol::{
    AgentStateView as State, CreateSession, GuardChoice, GuardPrompt, Reply, Request, Size,
};
use serde_json::{Value, json};
use tokio::sync::{Mutex, MutexGuard};

use crate::common::*;
use crate::fixtures::{self, Capture, Conversations, Recording};
use crate::isolation::{Isolation, copy_dir};
use crate::template;

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
const BRAIN: &str = ".gemini/antigravity-cli/brain";

static SERIAL: Mutex<()> = Mutex::const_new(());
static SKIPPED: std::sync::Once = std::sync::Once::new();

fn enabled(var: &str) -> bool {
    std::env::var(var).is_ok_and(|value| value == "1")
}

pub fn is_trust_screen(lower: &str) -> bool {
    lower.contains("trust")
}

pub fn report(text: &str) {
    let _ = std::io::stderr().write_all(text.as_bytes());
}

fn find_agy() -> PathBuf {
    std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join("agy"))
        .find(|path| path.is_file())
        .expect("ORCH_REAL_AGY=1 needs agy on PATH")
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
    argv: &[OsString],
) -> Held {
    let output = command
        .env("ORCH_RUNTIME_DIR", runtime)
        .args(["hold", "--session", session, "--runtime-dir"])
        .arg(runtime)
        .arg("--cwd")
        .arg(cwd)
        .arg("--")
        .args(argv)
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

fn capture_command(event: &str, answer: &str) -> String {
    format!(
        r#"d="$ORCH_REAL_AGY_CAPTURE/$ORCH_SESSION"; if [ -n "$ORCH_REAL_AGY_CAPTURE" ] && [ -n "$ORCH_SESSION" ]; then mkdir -p "$d" && cat > "$d/{event}-$(date +%s%N).json"; else cat > /dev/null; fi; printf '%s' '{answer}'"#
    )
}

fn add_capture_hooks(home: &Path, record: bool) {
    let path = home.join(".gemini/config/hooks.json");
    let mut hooks: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let hook = |event: &str, answer: &str| json!({ "type": "command", "command": capture_command(event, answer) });
    let mut capture = json!({
        "PostToolUse": [{ "matcher": "*", "hooks": [hook("PostToolUse", "{}")] }],
        "PreInvocation": [hook("PreInvocation", "{}")],
        "PostInvocation": [hook("PostInvocation", "{}")],
        "Stop": [hook("Stop", "{}")],
    });
    if record {
        let ask = hook("PreToolUse", r#"{"decision":"ask"}"#);
        capture["PreToolUse"] = json!([{ "matcher": "*", "hooks": [ask] }]);
    }
    hooks[CAPTURE_HOOK] = capture;
    std::fs::write(path, serde_json::to_string_pretty(&hooks).unwrap()).unwrap();
}

fn hostname() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|name| name.trim().to_string())
}

pub struct RealAgy {
    pub env: Env,
    pub agy: PathBuf,
    isolation: Isolation,
    captures: PathBuf,
    record: bool,
    _serial: MutexGuard<'static, ()>,
}

impl RealAgy {
    pub async fn new() -> Option<Self> {
        if !enabled(GATE) {
            SKIPPED.call_once(|| {
                report(&format!(
                    "real-agy suite skipped: set {GATE}=1 to run it against a real, signed-in agy\n"
                ))
            });
            return None;
        }
        let serial = SERIAL.lock().await;
        let agy = find_agy();
        let template = template::prepare(&agy).await;
        let env = Env::new();
        for dir in ["home", "state", "config"] {
            copy_dir(&template.join(dir), &env.path(dir));
        }
        env.write_config(&format!(
            "[defaults.agents.antigravity]\nbinary = {:?}\n",
            agy.display().to_string()
        ));
        let record = enabled(RECORD);
        add_capture_hooks(&env.path("home"), record);
        let captures = env.path("captures");
        std::fs::create_dir_all(&captures).unwrap();
        Some(Self {
            isolation: Isolation {
                home: env.path("home"),
                config: env.path("config"),
                state: env.path("state"),
            },
            env,
            agy,
            captures,
            record,
            _serial: serial,
        })
    }

    fn isolate<'c>(&self, command: &'c mut Command) -> &'c mut Command {
        self.isolation
            .apply(command)
            .env(CAPTURE_ENV, &self.captures)
    }

    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        let mut command = Command::new(program);
        self.isolate(&mut command);
        command
    }

    pub async fn start_daemon(&self) -> Daemon {
        let mut command = self.env.daemon_command(Duration::from_secs(3600));
        self.isolate(&mut command);
        self.env.spawn_daemon(&mut command).await
    }

    pub async fn hold(&self, session: &str, cwd: &Path, argv: &[OsString]) -> Held {
        let mut command = self.env.orch();
        self.isolate(&mut command);
        hold(command, &self.env.runtime_dir(), session, cwd, argv).await
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

    pub fn captures(&self, id: &SessionId) -> Vec<Capture> {
        fixtures::read_captures(&self.captures.join(id.as_str()))
    }

    pub fn transcript(&self, conversation: &str) -> String {
        let path = self
            .env
            .path("home")
            .join(BRAIN)
            .join(conversation)
            .join(".system_generated/logs/transcript_full.jsonl");
        std::fs::read_to_string(path).unwrap_or_default()
    }

    pub fn conversations(&self, client: &TestClient, id: &SessionId) -> Conversations {
        let root = client.sessions[id]
            .conversation
            .clone()
            .expect("agy's statusline names the Session's Conversation");
        let subagents = fixtures::spawned_subagents(&self.transcript(&root));
        Conversations { root, subagents }
    }

    pub fn record(&self, client: &TestClient, id: &SessionId, owner: Recording) {
        if !self.record {
            return;
        }
        let view = &client.sessions[id];
        let root = self.env.path("");
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
            (root.display().to_string(), "/home/dev/".into()),
        ];
        if let Ok(canonical) = root.canonicalize() {
            replacements.push((format!("{}/", canonical.display()), "/home/dev/".into()));
        }
        let mut forbidden = vec!["/tmp/".to_string()];
        if let Ok(home) = std::env::var("HOME") {
            replacements.push((home.clone(), "/home/dev".into()));
            forbidden.push(home);
        }
        forbidden.extend(std::env::var("USER").ok().filter(|user| user != "dev"));
        forbidden.extend(hostname());
        let written = fixtures::record(
            &self.captures.join(id.as_str()),
            &self.conversations(client, id),
            owner,
            replacements,
            forbidden,
        );
        report(&format!("re-recorded {written:?}\n"));
    }
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
