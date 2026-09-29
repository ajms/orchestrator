use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use orch_holder::{
    AgentExit, FromHolder, Hello, HolderClient, HolderEvent, ScreenSnapshot, ToHolder,
};
use tempfile::TempDir;

pub const WAIT: Duration = Duration::from_secs(10);

pub struct Sandbox {
    dir: TempDir,
}

impl Sandbox {
    pub fn new() -> Self {
        let dir = tempfile::Builder::new().prefix("oh").tempdir().unwrap();
        for sub in ["run", "home", "work"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        Self { dir }
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.path("run")
    }

    pub fn claude_settings(&self) -> PathBuf {
        self.path("home/.claude/settings.json")
    }

    pub fn write_claude_settings(&self, json: &str) {
        self.write_settings(self.claude_settings(), json);
    }

    pub fn write_settings(&self, path: PathBuf, json: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, json).unwrap();
    }

    pub fn orch(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_orch"));
        command
            .env("HOME", self.path("home"))
            .env("ORCH_RUNTIME_DIR", self.runtime_dir())
            .env(
                "ORCH_CLAUDE_MANAGED_SETTINGS",
                self.path("managed-settings.json"),
            )
            .current_dir(self.path("work"))
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("ORCH_HOLDER_SOCKET")
            .env_remove("XDG_RUNTIME_DIR")
            .stdin(Stdio::null());
        command
    }

    pub fn hold(&self, session: &str, script: &str) -> Held {
        self.hold_with(session, script, &[])
    }

    pub fn hold_with(&self, session: &str, script: &str, extra: &[&str]) -> Held {
        let script_path = self.path(&format!("{session}.script"));
        std::fs::write(&script_path, script).unwrap();
        let output = self
            .orch()
            .args(["hold", "--session", session, "--runtime-dir"])
            .arg(self.runtime_dir())
            .arg("--cwd")
            .arg(self.path("work"))
            .args(extra)
            .arg("--")
            .arg(env!("CARGO_BIN_EXE_orch"))
            .args(["fake-agent", "--script"])
            .arg(&script_path)
            .output()
            .unwrap();
        assert!(output.status.success(), "orch hold failed: {output:?}");
        let pid = String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        Held {
            pid,
            socket: orch_holder::socket_path(
                &self.runtime_dir(),
                &orch_core::SessionId::parse(session).unwrap(),
            ),
        }
    }
}

pub fn run_with_stdin(command: &mut Command, stdin: &str) -> Output {
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
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

pub struct Held {
    pub pid: i32,
    pub socket: PathBuf,
}

impl Held {
    pub async fn attach(&self) -> (HolderClient, Hello) {
        let mut client = HolderClient::connect(&self.socket).await.unwrap();
        let hello = client.attach().await.unwrap();
        (client, hello)
    }

    pub fn is_alive(&self) -> bool {
        Path::new(&format!("/proc/{}", self.pid)).exists() && !is_zombie(self.pid)
    }

    pub async fn wait_gone(&self) {
        let deadline = Instant::now() + WAIT;
        while self.is_alive() {
            assert!(
                Instant::now() < deadline,
                "holder {} still running",
                self.pid
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .arg("-9")
            .arg(self.pid.to_string())
            .status();
    }
}

fn is_zombie(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map(|stat| {
            stat.split(") ")
                .nth(1)
                .is_some_and(|rest| rest.starts_with('Z'))
        })
        .unwrap_or(true)
}

pub async fn wait_for_screen(
    client: &mut HolderClient,
    predicate: impl Fn(&str) -> bool,
) -> ScreenSnapshot {
    let deadline = Instant::now() + WAIT;
    loop {
        let snapshot = client.snapshot().await.unwrap();
        if predicate(&snapshot.text()) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "screen never matched:\n{}",
            snapshot.text()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

pub async fn next_matching<T>(
    client: &mut HolderClient,
    what: &str,
    mut pick: impl FnMut(FromHolder) -> Option<T>,
) -> T {
    let deadline = Instant::now() + WAIT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let message = tokio::time::timeout(remaining, client.recv())
            .await
            .unwrap_or_else(|_| panic!("no {what} in time"))
            .unwrap()
            .expect("holder closed the connection");
        if let Some(picked) = pick(message) {
            return picked;
        }
    }
}

pub async fn next_event(client: &mut HolderClient) -> (u64, HolderEvent) {
    next_matching(client, "event", |message| match message {
        FromHolder::Event { seq, event } => Some((seq, event)),
        _ => None,
    })
    .await
}

pub async fn next_hook_or_tap(client: &mut HolderClient) -> HolderEvent {
    loop {
        let (_, event) = next_event(client).await;
        if matches!(event, HolderEvent::Hook { .. } | HolderEvent::Tap { .. }) {
            return event;
        }
    }
}

pub async fn type_line(client: &mut HolderClient, line: &str) {
    send_bytes(client, format!("{line}\r").as_bytes()).await;
}

pub async fn send_bytes(client: &mut HolderClient, bytes: &[u8]) {
    client
        .send(&ToHolder::Input {
            bytes: bytes.to_vec(),
        })
        .await
        .unwrap();
}

pub async fn wait_for_exit(client: &mut HolderClient) -> AgentExit {
    loop {
        if let (_, HolderEvent::Exited(exit)) = next_event(client).await {
            return exit;
        }
    }
}

pub fn process_running(pid: u32) -> bool {
    let pid = pid as i32;
    Path::new(&format!("/proc/{pid}")).exists() && !is_zombie(pid)
}

pub fn holder_memory_kb(pid: i32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|value| value.trim().trim_end_matches("kB").trim().parse().ok())
        .unwrap()
}
