use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use nix::fcntl::{Flock, FlockArg};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use orch_core::SessionId;
use orch_holder::{Size, read_frame_async, write_frame_async};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

use crate::{
    DisplayVars, FromDaemon, OpenPane, PROTOCOL_VERSION, Reply, Request, RequestError, ToDaemon,
};

const DAEMON_SOCKET: &str = "daemon.sock";
const DAEMON_LOCK: &str = "daemon.lock";
pub const DEFAULT_SPAWN_WAIT: Duration = Duration::from_secs(5);
const SPAWN_POLL: Duration = Duration::from_millis(20);

pub fn daemon_socket(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(DAEMON_SOCKET)
}

#[derive(Debug)]
pub enum ConnectError {
    Io(io::Error),
    VersionMismatch {
        daemon_version: u32,
        message: String,
    },
}

impl From<io::Error> for ConnectError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "cannot reach the Daemon: {err}"),
            Self::VersionMismatch { message, .. } => f.write_str(message),
        }
    }
}

impl std::error::Error for ConnectError {}

struct Connection {
    reader: OwnedReadHalf,
    writer: OwnedWriteHalf,
}

pub async fn open_control(
    socket: &Path,
    display: &DisplayVars,
) -> Result<(OwnedReadHalf, OwnedWriteHalf), ConnectError> {
    Ok(Connection::open(socket, None, display.reported())
        .await?
        .into_split())
}

pub async fn open_pane(
    socket: &Path,
    pane: OpenPane,
) -> Result<(OwnedReadHalf, OwnedWriteHalf), ConnectError> {
    Ok(Connection::open(socket, Some(pane), None)
        .await?
        .into_split())
}

impl Connection {
    fn into_split(self) -> (OwnedReadHalf, OwnedWriteHalf) {
        (self.reader, self.writer)
    }

    async fn open(
        socket: &Path,
        pane: Option<OpenPane>,
        display: Option<DisplayVars>,
    ) -> Result<Self, ConnectError> {
        let (reader, writer) = UnixStream::connect(socket).await?.into_split();
        let mut connection = Self { reader, writer };
        connection
            .send(&ToDaemon::Hello {
                version: PROTOCOL_VERSION,
                pane,
                display,
            })
            .await?;
        match connection.recv().await? {
            Some(FromDaemon::Welcome { .. }) => Ok(connection),
            Some(FromDaemon::VersionMismatch {
                daemon_version,
                message,
            }) => Err(ConnectError::VersionMismatch {
                daemon_version,
                message,
            }),
            other => Err(ConnectError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected greeting from the Daemon: {other:?}"),
            ))),
        }
    }

    async fn send(&mut self, message: &ToDaemon) -> io::Result<()> {
        write_frame_async(&mut self.writer, message).await
    }

    async fn recv(&mut self) -> io::Result<Option<FromDaemon>> {
        read_frame_async(&mut self.reader).await
    }
}

pub struct Client {
    connection: Connection,
    next_id: u64,
    backlog: VecDeque<FromDaemon>,
}

impl Client {
    pub async fn connect(socket: &Path) -> Result<Self, ConnectError> {
        Ok(Self {
            connection: Connection::open(socket, None, None).await?,
            next_id: 0,
            backlog: VecDeque::new(),
        })
    }

    pub async fn request(&mut self, request: Request) -> io::Result<Result<Reply, RequestError>> {
        self.next_id += 1;
        let id = self.next_id;
        self.connection
            .send(&ToDaemon::Request { id, request })
            .await?;
        loop {
            match self.connection.recv().await? {
                Some(FromDaemon::Response {
                    id: answered,
                    result,
                }) if answered == id => {
                    return Ok(result);
                }
                Some(other) => self.backlog.push_back(other),
                None => return Err(io::ErrorKind::UnexpectedEof.into()),
            }
        }
    }

    #[doc(hidden)]
    pub fn take_backlog(&mut self) -> Vec<FromDaemon> {
        self.backlog.drain(..).collect()
    }

    pub async fn recv(&mut self) -> io::Result<Option<FromDaemon>> {
        match self.backlog.pop_front() {
            Some(message) => Ok(Some(message)),
            None => self.connection.recv().await,
        }
    }
}

pub struct Pane {
    connection: Connection,
}

impl Pane {
    pub async fn open(
        socket: &Path,
        session: &SessionId,
        size: Size,
    ) -> Result<Self, ConnectError> {
        let pane = OpenPane {
            session: session.clone(),
            size,
        };
        let connection = Connection::open(socket, Some(pane), None).await?;
        Ok(Self { connection })
    }

    pub async fn recv(&mut self) -> io::Result<Option<FromDaemon>> {
        self.connection.recv().await
    }

    pub async fn input(&mut self, bytes: impl Into<Vec<u8>>) -> io::Result<()> {
        let bytes = bytes.into();
        self.connection.send(&ToDaemon::Input { bytes }).await
    }

    pub async fn paste(&mut self, text: impl Into<String>) -> io::Result<()> {
        let text = text.into();
        self.connection.send(&ToDaemon::Paste { text }).await
    }

    pub async fn resize(&mut self, size: Size) -> io::Result<()> {
        self.connection.send(&ToDaemon::Resize(size)).await
    }
}

pub async fn connect_or_spawn(
    socket: &Path,
    spawn: impl FnOnce() -> io::Result<()>,
    wait: Duration,
) -> Result<Client, ConnectError> {
    match Client::connect(socket).await {
        Err(ConnectError::Io(err)) if daemon_absent(&err) => {}
        connected => return connected,
    }
    spawn()?;
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        match Client::connect(socket).await {
            Err(ConnectError::Io(err))
                if daemon_absent(&err) && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(SPAWN_POLL).await;
            }
            connected => return connected,
        }
    }
}

fn daemon_absent(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

pub fn spawn_detached(command: Command) -> io::Result<()> {
    let mut command = tokio::process::Command::from(command);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe and touches no memory of the parent.
    unsafe {
        command.pre_exec(|| nix::unistd::setsid().map(drop).map_err(io::Error::from));
    }
    command.spawn().map(drop)
}

pub fn daemon_command(orch_program: &Path) -> Command {
    let mut command = Command::new(orch_program);
    command.arg("daemon");
    command
}

pub fn daemon_lock(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(DAEMON_LOCK)
}

pub async fn connect_or_spawn_daemon(
    runtime_dir: &Path,
    orch_program: &Path,
) -> Result<Client, ConnectError> {
    let spawn = || spawn_detached(daemon_command(orch_program));
    connect_or_spawn(&daemon_socket(runtime_dir), spawn, DEFAULT_SPAWN_WAIT).await
}

pub const DAEMON_UNIT: &str = "orch-daemon.service";

pub async fn restart_running_daemon(
    runtime_dir: &Path,
    orch_program: &Path,
) -> Result<Client, ConnectError> {
    if daemon_under_systemd(runtime_dir) {
        restart_unit()?;
        let socket = daemon_socket(runtime_dir);
        return connect_or_spawn(&socket, || Ok(()), DEFAULT_SPAWN_WAIT).await;
    }
    let spawn = || spawn_detached(daemon_command(orch_program));
    restart_daemon(runtime_dir, spawn, DEFAULT_SPAWN_WAIT).await
}

fn restart_unit() -> io::Result<()> {
    let status = Command::new("systemctl")
        .args(["--user", "restart", DAEMON_UNIT])
        .stdin(Stdio::null())
        .status()?;
    match status.success() {
        true => Ok(()),
        false => Err(io::Error::other(format!(
            "systemctl --user restart {DAEMON_UNIT} failed ({status})"
        ))),
    }
}

fn daemon_under_systemd(runtime_dir: &Path) -> bool {
    let Ok(holder) = std::fs::read_to_string(daemon_lock(runtime_dir)) else {
        return false;
    };
    let Ok(pid) = holder.trim().parse::<u32>() else {
        return false;
    };
    std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
        .is_ok_and(|cgroup| in_daemon_unit(&cgroup))
}

fn in_daemon_unit(cgroup: &str) -> bool {
    cgroup
        .lines()
        .any(|line| line.trim_end().ends_with(&format!("/{DAEMON_UNIT}")))
}

pub async fn restart_daemon(
    runtime_dir: &Path,
    spawn: impl FnOnce() -> io::Result<()>,
    wait: Duration,
) -> Result<Client, ConnectError> {
    stop_daemon(runtime_dir, wait).await?;
    connect_or_spawn(&daemon_socket(runtime_dir), spawn, wait).await
}

pub async fn stop_daemon(runtime_dir: &Path, wait: Duration) -> io::Result<()> {
    let lock = daemon_lock(runtime_dir);
    let deadline = tokio::time::Instant::now() + wait;
    let mut signal = Signal::SIGTERM;
    loop {
        let file = match File::open(&lock) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err),
        };
        let mut holder = String::new();
        let file = match Flock::lock(file, FlockArg::LockSharedNonblock) {
            Ok(_released) => return Ok(()),
            Err((mut file, _)) => {
                file.read_to_string(&mut holder)?;
                file
            }
        };
        drop(file);
        if let Ok(pid) = holder.trim().parse::<i32>() {
            let _ = kill(Pid::from_raw(pid), signal);
        }
        if tokio::time::Instant::now() >= deadline {
            if signal == Signal::SIGKILL {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the running Daemon did not stop",
                ));
            }
            signal = Signal::SIGKILL;
        }
        tokio::time::sleep(SPAWN_POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_daemon_in_the_units_cgroup_runs_under_systemd() {
        let cgroup =
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/orch-daemon.service\n";
        assert!(in_daemon_unit(cgroup));
    }

    #[test]
    fn a_daemon_spawned_from_a_terminal_does_not() {
        let cgroup = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/gnome-terminal-server.service\n";
        assert!(!in_daemon_unit(cgroup));
        assert!(!in_daemon_unit("0::/user.slice/session-2.scope\n"));
    }
}
