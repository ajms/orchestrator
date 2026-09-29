#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use orch_core::SessionId;
use orch_protocol::{
    Client, CreateSession, FromDaemon, Pane, Reply, Request, RequestError, SessionView, Size,
    daemon_socket,
};
use tempfile::TempDir;

pub const WAIT: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(20);
pub const PANE: Size = Size {
    rows: 24,
    cols: 100,
};

const GIT_ENV: [(&str, &str); 6] = [
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
    ("GIT_AUTHOR_NAME", "Test"),
    ("GIT_AUTHOR_EMAIL", "test@example.com"),
    ("GIT_COMMITTER_NAME", "Test"),
    ("GIT_COMMITTER_EMAIL", "test@example.com"),
];

pub struct Env {
    dir: TempDir,
}

impl Env {
    pub fn new() -> Self {
        let dir = tempfile::Builder::new().prefix("od").tempdir().unwrap();
        for sub in ["run", "state", "config/orchestrator", "home", "repos"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        let env = Self { dir };
        env.write_config("");
        env
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.path("run")
    }

    pub fn socket(&self) -> PathBuf {
        daemon_socket(&self.runtime_dir())
    }

    pub fn write_config(&self, extra: &str) {
        self.write_config_with_agent(extra, env!("CARGO_BIN_EXE_orch"));
    }

    pub fn write_config_with_agent(&self, extra: &str, binary: &str) {
        let config = format!(
            "{extra}\n[defaults.agent]\nbinary = {binary:?}\nargs = [\"fake-agent\", \"--\"]\n"
        );
        std::fs::write(self.path("config/orchestrator/config.toml"), config).unwrap();
    }

    pub fn orch(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_orch"));
        command
            .env("HOME", self.path("home"))
            .env("XDG_CONFIG_HOME", self.path("config"))
            .env("XDG_STATE_HOME", self.path("state"))
            .env("ORCH_RUNTIME_DIR", self.runtime_dir())
            .env(
                "ORCH_CLAUDE_MANAGED_SETTINGS",
                self.path("managed-settings.json"),
            )
            .env_remove("XDG_RUNTIME_DIR")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("ORCH_HOLDER_SOCKET")
            .env_remove("ORCH_SESSION")
            .envs(GIT_ENV)
            .current_dir(self.path("home"))
            .stdin(Stdio::null());
        command
    }

    pub fn daemon_command(&self, idle_timeout: Duration) -> Command {
        let mut command = self.orch();
        command
            .arg("daemon")
            .arg("--idle-timeout-ms")
            .arg(idle_timeout.as_millis().to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        command
    }

    pub async fn start_daemon(&self) -> Daemon {
        self.start_daemon_with(Duration::from_secs(600)).await
    }

    pub async fn start_daemon_with(&self, idle_timeout: Duration) -> Daemon {
        let child = self.daemon_command(idle_timeout).spawn().unwrap();
        let daemon = Daemon { child };
        wait_until("daemon socket", || self.socket().exists()).await;
        daemon
    }

    pub async fn client(&self) -> TestClient {
        let deadline = Instant::now() + WAIT;
        loop {
            match Client::connect(&self.socket()).await {
                Ok(client) => return TestClient::from(client),
                Err(err) => assert!(Instant::now() < deadline, "cannot connect: {err}"),
            }
            tokio::time::sleep(POLL).await;
        }
    }

    pub async fn pane(&self, session: &SessionId, size: Size) -> PaneView {
        let pane = Pane::open(&self.socket(), session, size).await.unwrap();
        PaneView::new(pane)
    }

    pub fn repo(&self, name: &str) -> PathBuf {
        let root = self.path("repos").join(name);
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        commit(&root, "README.md", "hello\n");
        root
    }

    pub fn holder_socket(&self, session: &SessionId) -> PathBuf {
        orch_holder::socket_path(&self.runtime_dir(), session)
    }

    pub fn processes(&self) -> Vec<i32> {
        let marker = format!("ORCH_RUNTIME_DIR={}", self.runtime_dir().display());
        let me = std::process::id() as i32;
        std::fs::read_dir("/proc")
            .unwrap()
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<i32>().ok())
            .filter(|pid| *pid != me)
            .filter(|pid| {
                std::fs::read(format!("/proc/{pid}/environ")).is_ok_and(|environ| {
                    environ
                        .split(|byte| *byte == 0)
                        .any(|var| var == marker.as_bytes())
                })
            })
            .filter(|pid| !is_zombie(*pid))
            .collect()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let deadline = Instant::now() + WAIT;
        let mut quiet_scans = 0;
        while quiet_scans < 10 && Instant::now() < deadline {
            let alive = self.processes();
            quiet_scans = if alive.is_empty() { quiet_scans + 1 } else { 0 };
            for pid in alive {
                kill(pid, "-KILL");
            }
            std::thread::sleep(POLL);
        }
    }
}

pub fn kill(pid: i32, signal: &str) {
    let _ = Command::new("kill")
        .arg(signal)
        .arg(pid.to_string())
        .stderr(Stdio::null())
        .status();
}

pub fn is_zombie(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map(|stat| {
            stat.rsplit_once(") ")
                .is_some_and(|(_, rest)| rest.starts_with('Z'))
        })
        .unwrap_or(true)
}

pub struct Daemon {
    child: Child,
}

impl Daemon {
    pub fn pid(&self) -> i32 {
        self.child.id() as i32
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    pub async fn wait_exit(&mut self) -> std::process::ExitStatus {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < deadline, "daemon did not exit");
            tokio::time::sleep(POLL).await;
        }
    }

    pub fn has_exited(&mut self) -> bool {
        self.child.try_wait().unwrap().is_some()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.kill();
    }
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .envs(GIT_ENV)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim_end().into()
}

pub fn commit(dir: &Path, path: &str, content: &str) {
    let file = dir.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, content).unwrap();
    git(dir, &["add", path]);
    git(dir, &["commit", "-q", "-m", &format!("add {path}")]);
}

pub async fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(POLL).await;
    }
}

pub struct TestClient {
    pub client: Client,
    pub sessions: HashMap<SessionId, SessionView>,
    pub received_list: bool,
    pub history: Vec<SessionView>,
}

impl From<Client> for TestClient {
    fn from(client: Client) -> Self {
        Self {
            client,
            sessions: HashMap::new(),
            received_list: false,
            history: Vec::new(),
        }
    }
}

impl TestClient {
    pub fn holder_pid(&self, session: &SessionId) -> Option<i32> {
        self.sessions
            .get(session)
            .and_then(|view| view.holder_pid)
            .map(|pid| pid as i32)
    }

    pub async fn request(&mut self, request: Request) -> Result<Reply, RequestError> {
        tokio::time::timeout(WAIT, self.client.request(request))
            .await
            .expect("no response in time")
            .unwrap()
    }

    pub async fn create(&mut self, create: CreateSession) -> SessionId {
        match self.request(Request::CreateSession(create)).await {
            Ok(Reply::Created { session }) => session,
            other => panic!("create failed: {other:?}"),
        }
    }

    pub async fn next(&mut self) -> FromDaemon {
        let message = tokio::time::timeout(WAIT, self.client.recv())
            .await
            .expect("no message from the Daemon in time")
            .unwrap()
            .expect("Daemon closed the connection");
        self.apply(&message);
        message
    }

    fn apply(&mut self, message: &FromDaemon) {
        match message {
            FromDaemon::Sessions { sessions } => {
                self.received_list = true;
                self.sessions = sessions
                    .iter()
                    .map(|view| (view.id.clone(), view.clone()))
                    .collect();
            }
            FromDaemon::SessionChanged { session } => {
                self.history.push(*session.clone());
                self.sessions.insert(session.id.clone(), *session.clone());
            }
            _ => {}
        }
    }

    pub async fn session_list(&mut self) -> Vec<SessionView> {
        while !self.received_list {
            self.next().await;
        }
        self.sessions.values().cloned().collect()
    }

    pub async fn until(
        &mut self,
        session: &SessionId,
        what: &str,
        predicate: impl Fn(&SessionView) -> bool,
    ) -> SessionView {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(view) = self.sessions.get(session)
                && predicate(view)
            {
                return view.clone();
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let message = tokio::time::timeout(remaining, self.client.recv()).await;
            let Ok(message) = message else {
                panic!(
                    "Session never became {what}; last seen: {:#?}",
                    self.sessions.get(session)
                );
            };
            let message = message.unwrap().expect("Daemon closed the connection");
            self.apply(&message);
        }
    }
}

pub struct PaneView {
    pub pane: Pane,
    pub parser: vt100::Parser,
    pub resizes: Vec<Size>,
    pub closed: Option<String>,
}

impl PaneView {
    fn new(pane: Pane) -> Self {
        Self {
            pane,
            parser: vt100::Parser::new(1, 1, 0),
            resizes: Vec::new(),
            closed: None,
        }
    }

    pub fn text(&self) -> String {
        self.parser.screen().contents()
    }

    pub fn size(&self) -> Size {
        let (rows, cols) = self.parser.screen().size();
        Size { rows, cols }
    }

    async fn pump(&mut self, remaining: Duration) -> bool {
        let Ok(message) = tokio::time::timeout(remaining, self.pane.recv()).await else {
            return false;
        };
        match message.unwrap() {
            Some(FromDaemon::Screen(snapshot)) => self.parser = snapshot.restore(0),
            Some(FromDaemon::Output { bytes }) => self.parser.process(&bytes),
            Some(FromDaemon::Resized(size)) => {
                self.resizes.push(size);
                self.parser.screen_mut().set_size(size.rows, size.cols);
            }
            Some(FromDaemon::PaneClosed { reason }) => self.closed = Some(reason),
            Some(other) => panic!("unexpected pane message {other:?}"),
            None => self.closed = Some("eof".into()),
        }
        true
    }

    pub async fn wait_for(&mut self, what: &str, predicate: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + WAIT;
        while !predicate(self) {
            assert!(
                self.closed.is_none(),
                "pane closed ({:?}) before {what}:\n{}",
                self.closed,
                self.text()
            );
            let remaining = deadline.saturating_duration_since(Instant::now());
            if !self.pump(remaining).await {
                panic!("pane never showed {what}; screen:\n{}", self.text());
            }
        }
    }

    pub async fn wait_for_text(&mut self, needle: &str) {
        self.wait_for(needle, |pane| pane.text().contains(needle))
            .await;
    }

    pub async fn type_line(&mut self, line: &str) {
        self.pane
            .input(format!("{line}\r").into_bytes())
            .await
            .unwrap();
    }

    pub async fn hook(&mut self, payload: &str) {
        self.type_line(&format!("hook {payload}")).await;
    }
}

pub fn hook(event: &str, extra: &str) -> String {
    let extra = if extra.is_empty() {
        String::new()
    } else {
        format!(",{extra}")
    };
    format!(r#"{{"hook_event_name":"{event}","session_id":"conv-1"{extra}}}"#)
}

pub async fn running_session(env: &Env, client: &mut TestClient, prompt: &str) -> SessionId {
    let repo = env.path("repos/app");
    if !repo.exists() {
        env.repo("app");
    }
    let id = client.create(CreateSession::new(&repo, prompt)).await;
    client
        .until(&id, "running", |view| view.agent.is_some())
        .await;
    id
}

pub async fn settled(client: &mut TestClient) {
    let barrier = orch_protocol::Request::View {
        session: None,
        focused: false,
    };
    client.request(barrier).await.unwrap();
}
