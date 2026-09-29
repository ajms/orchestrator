mod agents;
mod cleanup;
mod client;
mod discard;
mod draft;
mod holder;
mod landing;
mod lifecycle;
mod notify;
mod outbox;
mod pane;
mod pr;
mod rate_limits;
mod recency;
mod reconcile;
mod repos;
mod setup;
mod state;
mod store;
mod subprocess;
mod usage;

use std::fs::File;
use std::io::{self, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use nix::fcntl::{Flock, FlockArg};
use orch_config::ConfigLoader;
use orch_protocol::{daemon_lock, daemon_socket};
use orch_store::Store;
use tokio::net::UnixListener;
use tokio::signal::unix::{SignalKind, signal};

use crate::state::Daemon;
use crate::store::StoreHandle;

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub runtime_dir: PathBuf,
    pub state_db: PathBuf,
    pub sessions_dir: PathBuf,
    pub loader: ConfigLoader,
    pub orch_program: PathBuf,
    pub idle_timeout: Option<Duration>,
    pub pr_poll_interval: Duration,
    pub reconcile_interval: Duration,
    pub notifications: NotificationTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationTarget {
    Desktop,
    Log(PathBuf),
}

impl DaemonConfig {
    pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
    pub const DEFAULT_PR_POLL: Duration = Duration::from_secs(60);
    pub const DEFAULT_RECONCILE: Duration = Duration::from_secs(3 * 60);
}

pub fn run(config: DaemonConfig) -> io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&config.runtime_dir)?;
    let Some(_lock) = lock(&config.runtime_dir)? else {
        return Ok(());
    };
    let store = StoreHandle::spawn(Store::open(&config.state_db).map_err(io::Error::other)?);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let socket = daemon_socket(&config.runtime_dir);
    let result = runtime.block_on(async {
        let listener = bind(&socket)?;
        let daemon = Daemon::start(config, store).await;
        serve(daemon, listener).await
    });
    let _ = std::fs::remove_file(&socket);
    runtime.shutdown_timeout(Duration::from_millis(200));
    result
}

fn lock(runtime_dir: &Path) -> io::Result<Option<Flock<File>>> {
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(daemon_lock(runtime_dir))?;
    let lock = match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
        Ok(lock) => lock,
        Err((_, nix::errno::Errno::EWOULDBLOCK)) => return Ok(None),
        Err((_, errno)) => return Err(errno.into()),
    };
    lock.set_len(0)?;
    write!(&*lock, "{}", std::process::id())?;
    Ok(Some(lock))
}

fn bind(socket: &Path) -> io::Result<UnixListener> {
    if socket.exists() {
        if StdUnixStream::connect(socket).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("a Daemon already listens on {}", socket.display()),
            ));
        }
        std::fs::remove_file(socket)?;
    }
    UnixListener::bind(socket)
}

async fn serve(daemon: Arc<Daemon>, listener: UnixListener) -> io::Result<()> {
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::spawn(daemon.clone().watch_idle());
    tokio::spawn(daemon.clone().tick_stalled());
    tokio::spawn(daemon.clone().poll_prs());
    tokio::spawn(daemon.clone().reconcile_loop());
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                tokio::spawn(client::serve(daemon.clone(), stream));
            }
            () = daemon.shutdown.notified() => return Ok(()),
            _ = terminate.recv() => return Ok(()),
        }
    }
}
