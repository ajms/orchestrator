#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use orch_core::SessionId;
use orch_git::ENV_REDIRECTING_GIT;
use orch_protocol::{
    Client, CreateSession, DisplayVars, Fix, FromDaemon, Pane, ReconcileReport, Reply, Request,
    RequestError, SessionView, Size, SubagentTranscript, daemon_socket, open_control,
};
use tempfile::TempDir;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

pub const UNSANDBOXED: &str = "[defaults]\nsandbox = false\n";
pub const OPEN_PRESET: &str = "[defaults.presets.open]\nsandbox = false\n";
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

const GIT_CONFIG_INJECTED: [&str; 2] = ["GIT_CONFIG_PARAMETERS", "GIT_CONFIG_COUNT"];

fn hermetic(command: &mut Command) -> &mut Command {
    for key in ENV_REDIRECTING_GIT.iter().chain(&GIT_CONFIG_INJECTED) {
        command.env_remove(key);
    }
    command.envs(GIT_ENV)
}

pub struct Env {
    dir: TempDir,
}

impl Env {
    pub fn new() -> Self {
        let dir = tempfile::Builder::new().prefix("od").tempdir().unwrap();
        for sub in [
            "run",
            "state",
            "config/orchestrator",
            "home",
            "repos",
            "bin",
        ] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        let env = Self { dir };
        env.write_config("");
        env.fake_bwrap();
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

    pub fn notify_log(&self) -> PathBuf {
        self.path("notify.log")
    }

    pub fn notifications(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(self.notify_log())
            .unwrap_or_default()
            .split_inclusive('\n')
            .filter(|line| line.ends_with('\n'))
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    pub async fn until_notified(
        &self,
        what: &str,
        ready: impl Fn(&[serde_json::Value]) -> bool,
    ) -> Vec<serde_json::Value> {
        wait_until(what, || ready(&self.notifications())).await;
        self.notifications()
    }

    pub fn click_notification(&self, key: &str) {
        use std::io::Write;
        let mut clicks = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path("notify.log.clicks"))
            .unwrap();
        writeln!(clicks, "{key}").unwrap();
    }

    pub fn orch(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_orch"));
        hermetic(&mut command)
            .env("HOME", self.path("home"))
            .env("XDG_CONFIG_HOME", self.path("config"))
            .env("XDG_STATE_HOME", self.path("state"))
            .env("ORCH_RUNTIME_DIR", self.runtime_dir())
            .env(
                "ORCH_CLAUDE_MANAGED_SETTINGS",
                self.path("managed-settings.json"),
            )
            .env("PATH", self.search_path())
            .env_remove("XDG_RUNTIME_DIR")
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("ORCH_HOLDER_SOCKET")
            .env_remove("ORCH_SESSION")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DISPLAY")
            .current_dir(self.path("home"))
            .stdin(Stdio::null());
        command
    }

    fn search_path(&self) -> String {
        let inherited = std::env::var("PATH").unwrap_or_default();
        format!("{}:{inherited}", self.path("bin").display())
    }

    pub fn daemon_command(&self, idle_timeout: Duration) -> Command {
        let mut command = self.orch();
        command
            .arg("daemon")
            .arg("--idle-timeout-ms")
            .arg(idle_timeout.as_millis().to_string())
            .arg("--notify-log")
            .arg(self.notify_log())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        command
    }

    pub async fn start_daemon(&self) -> Daemon {
        self.start_daemon_with(Duration::from_secs(600)).await
    }

    pub async fn start_daemon_with(&self, idle_timeout: Duration) -> Daemon {
        self.start_daemon_args(idle_timeout, &[]).await
    }

    pub async fn start_daemon_args(&self, idle_timeout: Duration, args: &[&str]) -> Daemon {
        let mut command = self.daemon_command(idle_timeout);
        command.args(args);
        self.spawn_daemon(&mut command).await
    }

    pub async fn spawn_daemon(&self, command: &mut Command) -> Daemon {
        let child = command.spawn().unwrap();
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

    pub async fn tui(&self, display: DisplayVars) -> (OwnedReadHalf, OwnedWriteHalf) {
        open_control(&self.socket(), &display).await.unwrap()
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

    pub fn remote(&self, repo: &Path) -> PathBuf {
        let remote = self.path("remote.git");
        git(
            &self.path("repos"),
            &["init", "-q", "--bare", remote.to_str().unwrap()],
        );
        git(repo, &["remote", "add", "origin", remote.to_str().unwrap()]);
        git(repo, &["push", "-q", "origin", "main"]);
        remote
    }

    pub fn github_remote(&self, repo: &Path) -> PathBuf {
        let remote = self.remote(repo);
        let github = "git@github.com:acme/app.git";
        git(repo, &["remote", "set-url", "origin", github]);
        let rewrite = format!("url.{}.insteadOf", remote.display());
        git(repo, &["config", &rewrite, github]);
        remote
    }

    pub fn fake_gh(&self) {
        let dir = self.path("gh");
        std::fs::create_dir_all(&dir).unwrap();
        let script = r#"#!/bin/sh
printf '%s\n' "$PWD" "$@" >> DIR/calls.log
case "$1 $2" in
  'pr create')
    [ -f DIR/slow ] && sleep 1
    [ -f DIR/fail-create ] && { echo 'gh: permission denied' >&2; exit 1; }
    echo https://github.com/acme/app/pull/42 ;;
  'pr view')
    [ -f DIR/fail-view ] && { echo 'GraphQL: Could not resolve to a PullRequest' >&2; exit 1; }
    cat DIR/view.json ;;
  'pr edit')
    if [ -f DIR/fail-edit ]; then echo 'gh: edit failed' >&2; exit 1; fi ;;
  'pr ready') ;;
  'issue create') echo https://github.com/acme/app/issues/8 ;;
  'issue comment'|'issue edit'|'issue close'|'issue reopen') ;;
  'issue view') echo "issue $3" ;;
  'api '*) echo '{"name":"app"}' ;;
  'pr list')
    [ -f DIR/fail-list ] && { echo 'HTTP 401: Bad credentials' >&2; exit 1; }
    cat DIR/list.json 2>/dev/null || echo '[]' ;;
  *) exit 1 ;;
esac
"#
        .replace("DIR", &dir.display().to_string());
        let gh = self.path("bin/gh");
        std::fs::write(&gh, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        self.gh_reports(
            r#"{"state":"OPEN","statusCheckRollup":[],"reviewDecision":"","comments":[],"reviews":[]}"#,
        );
    }

    pub fn fake_bwrap(&self) {
        let dir = self.path("bwrap");
        std::fs::create_dir_all(&dir).unwrap();
        let script = r#"#!/bin/sh
[ -f DIR/outer-fails ] && { echo 'bwrap: setting up uid map: Permission denied' >&2; exit 1; }
case " $* " in
  *" --unshare-user "*) [ -f DIR/nested-fails ] && { echo 'bwrap: No permissions to create a new namespace' >&2; exit 1; } ;;
  *" --json-status-fd "*) [ -f DIR/start-fails ] && { echo "bwrap: Can't find source path /gone" >&2; exit 1; } ;;
esac
while [ $# -gt 0 ] && [ "$1" != "--" ]; do
  case "$1" in
    --setenv) export "$2=$3"; shift 3 ;;
    *) shift ;;
  esac
done
shift
exec "$@"
"#
        .replace("DIR", &dir.display().to_string());
        let bwrap = self.path("bin/bwrap");
        std::fs::write(&bwrap, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bwrap, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    pub fn real_bwrap(&self) {
        std::fs::remove_file(self.path("bin/bwrap")).unwrap();
    }

    pub fn bwrap_switch(&self, name: &str, on: bool) {
        let flag = self.path("bwrap").join(name);
        match on {
            true => std::fs::write(flag, "").unwrap(),
            false => {
                let _ = std::fs::remove_file(flag);
            }
        }
    }

    pub fn gh_switch(&self, name: &str, on: bool) {
        let flag = self.path("gh").join(name);
        match on {
            true => std::fs::write(flag, "").unwrap(),
            false => {
                let _ = std::fs::remove_file(flag);
            }
        }
    }

    pub fn gh_reports(&self, json: &str) {
        let view = self.path("gh/view.json");
        let staged = self.path("gh/view.json.new");
        std::fs::write(&staged, json).unwrap();
        std::fs::rename(staged, view).unwrap();
    }

    pub fn gh_lists(&self, json: &str) {
        std::fs::write(self.path("gh/list.json"), json).unwrap();
    }

    pub fn gh_calls(&self) -> String {
        std::fs::read_to_string(self.path("gh/calls.log")).unwrap_or_default()
    }

    pub fn lose_state_db(&self) {
        let dir = self.path("state/orchestrator");
        for name in ["state.db", "state.db-wal", "state.db-shm"] {
            let _ = std::fs::remove_file(dir.join(name));
        }
    }

    pub fn hold(&self, session: &str, cwd: &Path) -> i32 {
        self.hold_with(session, cwd, &[])
    }

    pub fn hold_with(&self, session: &str, cwd: &Path, args: &[&str]) -> i32 {
        let output = self
            .orch()
            .args(["hold", "--session", session, "--runtime-dir"])
            .arg(self.runtime_dir())
            .arg("--cwd")
            .arg(cwd)
            .args(args)
            .args(["--", env!("CARGO_BIN_EXE_orch"), "fake-agent", "--"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap()
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
            .filter(|pid| !process_gone(*pid))
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

pub fn process_gone(pid: i32) -> bool {
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
    let output = hermetic(&mut Command::new("git"))
        .arg("-C")
        .arg(dir)
        .args(args)
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

#[derive(Debug, Clone, PartialEq)]
pub struct Ring {
    pub session: SessionId,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateLimits {
    pub five_hour: Option<f64>,
    pub seven_day: Option<f64>,
}

pub struct TestClient {
    pub client: Client,
    pub sessions: HashMap<SessionId, SessionView>,
    pub received_list: bool,
    pub history: Vec<SessionView>,
    pub removed: Vec<SessionId>,
    pub reports: Vec<ReconcileReport>,
    pub rings: Vec<Ring>,
    pub focused: Vec<SessionId>,
    pub rate_limits: Vec<RateLimits>,
    pub copies: Vec<(SessionId, String)>,
    pub notices: Vec<(SessionId, String)>,
    pub transcripts: Vec<SubagentTranscript>,
    session_in_view: Option<SessionId>,
    focused_terminal: bool,
}

impl From<Client> for TestClient {
    fn from(client: Client) -> Self {
        Self {
            client,
            sessions: HashMap::new(),
            received_list: false,
            history: Vec::new(),
            removed: Vec::new(),
            reports: Vec::new(),
            rings: Vec::new(),
            focused: Vec::new(),
            rate_limits: Vec::new(),
            copies: Vec::new(),
            notices: Vec::new(),
            transcripts: Vec::new(),
            session_in_view: None,
            focused_terminal: false,
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

    pub async fn reconcile(&mut self) -> ReconcileReport {
        match self.request(Request::Reconcile).await {
            Ok(Reply::Reconciled { report }) => *report,
            other => panic!("reconcile failed: {other:?}"),
        }
    }

    pub async fn fix(&mut self, fix: Fix) -> Result<Reply, RequestError> {
        self.request(Request::Fix { fix }).await
    }

    pub async fn until_removed(&mut self, session: &SessionId) {
        let deadline = Instant::now() + WAIT;
        while !self.removed.contains(session) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let Ok(message) = tokio::time::timeout(remaining, self.client.recv()).await else {
                panic!("Session {} was never removed", session.as_str());
            };
            let message = message.unwrap().expect("Daemon closed the connection");
            self.apply(&message);
        }
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
            FromDaemon::SessionRemoved { session } => {
                self.sessions.remove(session);
                self.removed.push(session.clone());
            }
            FromDaemon::Reconciled { report } => self.reports.push(*report.clone()),
            FromDaemon::Ring {
                session,
                title,
                body,
            } => self.rings.push(Ring {
                session: session.clone(),
                title: title.clone(),
                body: body.clone(),
            }),
            FromDaemon::Focus { session } => self.focused.push(session.clone()),
            FromDaemon::RateLimits {
                five_hour,
                seven_day,
            } => self.rate_limits.push(RateLimits {
                five_hour: *five_hour,
                seven_day: *seven_day,
            }),
            FromDaemon::Clipboard { session, text } => {
                self.copies.push((session.clone(), text.clone()))
            }
            FromDaemon::Notice { session, text, .. } => {
                self.notices.push((session.clone(), text.clone()))
            }
            FromDaemon::SubagentTranscript(transcript) => self.transcripts.push(transcript.clone()),
            _ => {}
        }
    }

    pub async fn until_notice(&mut self, session: &SessionId, text: &str) {
        self.until_received(text, |client| {
            client
                .notices
                .iter()
                .any(|(of, notice)| of == session && notice.contains(text))
        })
        .await;
    }

    pub async fn until_received(&mut self, what: &str, ready: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + WAIT;
        while !ready(self) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let Ok(message) = tokio::time::timeout(remaining, self.client.recv()).await else {
                panic!("never received {what}");
            };
            let message = message.unwrap().expect("Daemon closed the connection");
            self.apply(&message);
        }
    }

    pub async fn view(&mut self, session: Option<&SessionId>, focused: bool) {
        self.session_in_view = session.cloned();
        self.focused_terminal = focused;
        self.drain().await;
    }

    pub async fn drain(&mut self) {
        let view = Request::View {
            session: self.session_in_view.clone(),
            focused: self.focused_terminal,
        };
        self.request(view).await.unwrap();
        for message in self.client.take_backlog() {
            self.apply(&message);
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
    pub dropped: Vec<String>,
}

impl PaneView {
    fn new(pane: Pane) -> Self {
        Self {
            pane,
            parser: vt100::Parser::new(1, 1, 0),
            resizes: Vec::new(),
            closed: None,
            dropped: Vec::new(),
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
            Some(FromDaemon::InputDropped { reason }) => self.dropped.push(reason),
            Some(other) => panic!("unexpected pane message {other:?}"),
            None => self.closed = Some("eof".into()),
        }
        true
    }

    pub async fn wait_for(&mut self, what: &str, predicate: impl Fn(&Self) -> bool) {
        self.wait_for_within(WAIT, what, predicate).await;
    }

    pub async fn wait_for_within(
        &mut self,
        within: Duration,
        what: &str,
        predicate: impl Fn(&Self) -> bool,
    ) {
        let deadline = Instant::now() + within;
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

    // A resize turns soft wraps into line breaks, so a long line printed before it may be split.
    pub async fn wait_for_long_line(&mut self, needle: &str) {
        self.wait_for(needle, |pane| {
            pane.text().replace('\n', "").contains(needle)
        })
        .await;
    }

    pub async fn type_line(&mut self, line: &str) {
        self.pane
            .input(format!("{line}\r").into_bytes())
            .await
            .unwrap();
    }

    pub async fn report(&mut self, bytes: &str) {
        self.pane.report(bytes.as_bytes().to_vec()).await.unwrap();
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

pub async fn idle_session(
    env: &Env,
    client: &mut TestClient,
    prompt: &str,
) -> (SessionId, PaneView) {
    let id = running_session(env, client, prompt).await;
    let mut pane = env.pane(&id, PANE).await;
    pane.hook(&hook("SessionStart", r#""source":"startup""#))
        .await;
    client
        .until(&id, "Idle", |view| {
            view.agent == Some(orch_protocol::AgentStateView::Idle)
        })
        .await;
    (id, pane)
}

pub async fn settled(client: &mut TestClient) {
    let barrier = orch_protocol::Request::View {
        session: None,
        focused: false,
    };
    client.request(barrier).await.unwrap();
}

pub fn on_branch(repo: &Path, branch: &str) -> CreateSession {
    let mut create = CreateSession::new(repo, "Continue the feature");
    create.branch = Some(branch.into());
    create
}

pub fn feature_branch(repo: &Path) -> String {
    git(repo, &["checkout", "-q", "-b", "feature-x"]);
    commit(repo, "feature.txt", "feature\n");
    git(repo, &["checkout", "-q", "main"]);
    git(repo, &["rev-parse", "feature-x"])
}

pub async fn running_on(client: &mut TestClient, create: CreateSession) -> SessionId {
    let id = client.create(create).await;
    client
        .until(&id, "running", |view| view.agent.is_some())
        .await;
    id
}

pub async fn idle_on(env: &Env, client: &mut TestClient, create: CreateSession) -> SessionId {
    let id = running_on(client, create).await;
    let mut pane = env.pane(&id, PANE).await;
    pane.hook(&hook("SessionStart", r#""source":"startup""#))
        .await;
    client
        .until(&id, "Idle", |view| {
            view.agent == Some(orch_protocol::AgentStateView::Idle)
        })
        .await;
    id
}

pub async fn refusal(client: &mut TestClient, create: CreateSession) -> String {
    match client.request(Request::CreateSession(create)).await {
        Err(RequestError::Refused { message }) => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

pub fn branch_tip(repo: &Path, branch: &str) -> String {
    git(repo, &["rev-parse", &format!("refs/heads/{branch}")])
}
