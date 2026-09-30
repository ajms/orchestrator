use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::time::Duration;

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use orch_agent::GuardAnswer;
use orch_config::PortBlock;
use orch_core::SessionId;
use orch_term::keys::encode_paste;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::net::UnixListener;
use tokio::sync::{Notify, oneshot, watch};

use crate::connection::{ConnId, Outbox, ScreenFeed, capture, serve_connection};
use crate::event_log::EventLog;
use crate::guards::GuardTable;
use crate::paths::{HOLDER_SOCKET_ENV, SESSION_ENV};
use crate::{
    AgentExit, AgentStatus, Emulator, FromHolder, Hello, HolderEvent, PROTOCOL_VERSION, ScreenCopy,
    Size, ToHolder,
};

const FINAL_OUTPUT_GRACE: Duration = Duration::from_secs(2);
const KILL_GRACE: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct HoldConfig {
    pub session: SessionId,
    pub socket: PathBuf,
    pub cwd: PathBuf,
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub base: Option<String>,
    pub port_block: Option<PortBlock>,
    pub size: Size,
    pub guard_timeout: Duration,
    pub event_capacity: usize,
    pub scrollback: usize,
}

impl HoldConfig {
    pub const DEFAULT_GUARD_TIMEOUT: Duration = Duration::from_secs(10);
    pub const DEFAULT_EVENT_CAPACITY: usize = 10_000;
    pub const DEFAULT_SCROLLBACK: usize = 10_000;

    pub fn new(session: SessionId, socket: PathBuf, cwd: PathBuf, argv: Vec<String>) -> Self {
        Self {
            session,
            socket,
            cwd,
            argv,
            env: Vec::new(),
            base: None,
            port_block: None,
            size: Size::DEFAULT,
            guard_timeout: Self::DEFAULT_GUARD_TIMEOUT,
            event_capacity: Self::DEFAULT_EVENT_CAPACITY,
            scrollback: Self::DEFAULT_SCROLLBACK,
        }
    }
}

pub fn hold(config: HoldConfig) -> io::Result<()> {
    serve(config, || {})
}

pub(crate) fn serve(config: HoldConfig, ready: impl FnOnce()) -> io::Result<()> {
    let listener = bind(&config.socket)?;
    let shared = match spawn_agent(&config) {
        Ok(shared) => shared,
        Err(err) => {
            let _ = std::fs::remove_file(&config.socket);
            return Err(err);
        }
    };
    ready();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(accept_loop(listener, shared));
    let _ = std::fs::remove_file(&config.socket);
    result
}

fn bind(socket: &Path) -> io::Result<std::os::unix::net::UnixListener> {
    if let Some(dir) = socket.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    if socket.exists() {
        if StdUnixStream::connect(socket).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("a holder already listens on {}", socket.display()),
            ));
        }
        std::fs::remove_file(socket)?;
    }
    let listener = std::os::unix::net::UnixListener::bind(socket)?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

async fn accept_loop(
    listener: std::os::unix::net::UnixListener,
    shared: Arc<Shared>,
) -> io::Result<()> {
    let listener = UnixListener::from_std(listener)?;
    let mut next_conn = 0;
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                next_conn += 1;
                tokio::spawn(serve_connection(shared.clone(), stream, next_conn));
            }
            () = shared.shutdown.notified() => return Ok(()),
        }
    }
}

pub(crate) struct Shared {
    state: Mutex<State>,
    input: mpsc::Sender<Vec<u8>>,
    shutdown: Notify,
    exited: watch::Sender<bool>,
    guard_timeout: Duration,
}

struct State {
    session: SessionId,
    cwd: PathBuf,
    base: Option<String>,
    port_block: Option<PortBlock>,
    agent_pid: Option<u32>,
    emulator: Emulator,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    exit: Option<AgentExit>,
    log: EventLog,
    upstream: Option<(ConnId, Outbox)>,
    subscribers: HashMap<ConnId, ScreenFeed>,
    guards: GuardTable,
}

fn spawn_agent(config: &HoldConfig) -> io::Result<Arc<Shared>> {
    let pty = native_pty_system()
        .openpty(pty_size(config.size))
        .map_err(io::Error::other)?;
    let (program, args) = config
        .argv
        .split_first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty Agent argv"))?;
    let mut command = CommandBuilder::new(program);
    command.args(args);
    command.cwd(&config.cwd);
    command.env("TERM", "xterm-256color");
    command.env(SESSION_ENV, config.session.as_str());
    command.env(HOLDER_SOCKET_ENV, &config.socket);
    for (key, value) in &config.env {
        command.env(key, value);
    }
    let mut child = pty.slave.spawn_command(command).map_err(io::Error::other)?;
    drop(pty.slave);
    let mut reader = pty.master.try_clone_reader().map_err(io::Error::other)?;
    let mut writer = pty.master.take_writer().map_err(io::Error::other)?;
    let agent_pid = child.process_id();

    let (input, keystrokes) = mpsc::channel::<Vec<u8>>();
    let mut log = EventLog::new(config.event_capacity);
    log.push(HolderEvent::Spawned { pid: agent_pid });
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            session: config.session.clone(),
            cwd: config.cwd.clone(),
            base: config.base.clone(),
            port_block: config.port_block,
            agent_pid,
            emulator: Emulator::new(config.size, config.scrollback),
            master: pty.master,
            killer: child.clone_killer(),
            exit: None,
            log,
            upstream: None,
            subscribers: HashMap::new(),
            guards: GuardTable::default(),
        }),
        input,
        shutdown: Notify::new(),
        exited: watch::Sender::new(false),
        guard_timeout: config.guard_timeout,
    });

    std::thread::spawn(move || {
        for bytes in keystrokes {
            if writer
                .write_all(&bytes)
                .and_then(|()| writer.flush())
                .is_err()
            {
                break;
            }
        }
    });
    let (drained, output_done) = mpsc::channel::<()>();
    let output = shared.clone();
    std::thread::spawn(move || {
        let mut buf = [0; 16 * 1024];
        while let Ok(n @ 1..) = reader.read(&mut buf) {
            output.lock().output(&buf[..n]);
        }
        let _ = drained.send(());
    });
    let exit = shared.clone();
    std::thread::spawn(move || {
        let status = child.wait();
        let _ = output_done.recv_timeout(FINAL_OUTPUT_GRACE);
        let exit_info = match status {
            Ok(status) => AgentExit {
                code: status.exit_code(),
                signal: status.signal().map(String::from),
            },
            Err(err) => AgentExit {
                code: 1,
                signal: Some(err.to_string()),
            },
        };
        let mut state = exit.lock();
        state.exit = Some(exit_info.clone());
        state.record(HolderEvent::Exited(exit_info));
        drop(state);
        exit.exited.send_replace(true);
    });
    Ok(shared)
}

fn pty_size(size: Size) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn resync(&self, lagged: &AtomicBool) -> ScreenCopy {
        let state = self.lock();
        lagged.store(false, Ordering::Release);
        state.emulator.copy()
    }

    fn open_guard(self: &Arc<Self>, payload: String) -> oneshot::Receiver<GuardAnswer> {
        let (reply, answer) = oneshot::channel();
        let mut state = self.lock();
        if state.upstream.is_none() {
            state.record(HolderEvent::Hook {
                payload,
                guard: None,
            });
            let _ = reply.send(GuardAnswer::Ask);
            return answer;
        }
        let id = state.guards.open(reply);
        state.record(HolderEvent::Hook {
            payload,
            guard: Some(id),
        });
        drop(state);
        let shared = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(shared.guard_timeout).await;
            let mut state = shared.lock();
            if state.guards.is_unheld(id) {
                state.settle_guard(id, GuardAnswer::Ask);
            }
        });
        answer
    }

    pub(crate) fn disconnect(&self, conn: ConnId) {
        let mut state = self.lock();
        state.subscribers.remove(&conn);
        if state.upstream.as_ref().is_some_and(|(id, _)| *id == conn) {
            state.upstream = None;
            for id in state.guards.ids() {
                state.settle_guard(id, GuardAnswer::Ask);
            }
        }
    }

    fn signal_agent(&self, signal: Signal) {
        let mut state = self.lock();
        if state.exit.is_some() {
            return;
        }
        let delivered = state
            .agent_pid
            .and_then(|pid| i32::try_from(pid).ok())
            .is_some_and(|pid| killpg(Pid::from_raw(pid), signal).is_ok());
        if !delivered {
            let _ = state.killer.kill();
        }
    }

    async fn wait_exited(&self, limit: Duration) -> bool {
        let mut exited = self.exited.subscribe();
        tokio::time::timeout(limit, exited.wait_for(|exited| *exited))
            .await
            .is_ok()
    }

    async fn stop_agent(&self) {
        self.signal_agent(Signal::SIGHUP);
        if !self.wait_exited(KILL_GRACE).await {
            self.signal_agent(Signal::SIGKILL);
            self.wait_exited(FINAL_OUTPUT_GRACE * 2).await;
        }
    }
}

impl State {
    fn record(&mut self, event: HolderEvent) {
        let seq = self.log.push(event.clone());
        if let Some((_, upstream)) = &self.upstream {
            upstream.send(FromHolder::Event { seq, event });
        }
    }

    fn output(&mut self, bytes: &[u8]) {
        self.emulator.process(bytes);
        for text in self.emulator.take_copies() {
            if let Some((_, upstream)) = &self.upstream {
                upstream.send(FromHolder::Clipboard { text });
            }
        }
        self.subscribers.retain(|_, feed| {
            feed.push(FromHolder::Output {
                bytes: bytes.to_vec(),
            })
        });
    }

    fn settle_guard(&mut self, id: crate::GuardId, answer: GuardAnswer) {
        if let Some(reply) = self.guards.take(id) {
            self.log.settle_guard(id);
            let _ = reply.send(answer);
        }
    }

    fn hello(&self) -> Hello {
        Hello {
            version: PROTOCOL_VERSION,
            session: self.session.clone(),
            cwd: Some(self.cwd.clone()),
            base: self.base.clone(),
            port_block: self.port_block,
            holder_pid: std::process::id(),
            agent_pid: self.agent_pid,
            agent: match &self.exit {
                Some(exit) => AgentStatus::Exited(exit.clone()),
                None => AgentStatus::Running,
            },
        }
    }

    fn attach(&mut self, conn: ConnId, outbox: &Outbox) {
        if let Some((previous, superseded)) = self.upstream.take()
            && previous != conn
        {
            superseded.close_with(FromHolder::Superseded);
        }
        outbox.send(FromHolder::Hello(self.hello()));
        for (seq, event) in self.log.unacked() {
            outbox.send(FromHolder::Event {
                seq: *seq,
                event: event.clone(),
            });
        }
        self.upstream = Some((conn, outbox.clone()));
    }

    fn resize(&mut self, size: Size) {
        if self.master.resize(pty_size(size)).is_err() {
            return;
        }
        self.emulator.resize(size);
        self.subscribers
            .retain(|_, feed| feed.push(FromHolder::Resized(size)));
    }
}

pub(crate) async fn handle(
    shared: &Arc<Shared>,
    conn: ConnId,
    outbox: &Outbox,
    feed: &ScreenFeed,
    message: ToHolder,
) -> bool {
    match message {
        ToHolder::Attach { version } => {
            let mut state = shared.lock();
            if version != PROTOCOL_VERSION {
                outbox.send(FromHolder::Hello(state.hello()));
                return false;
            }
            state.attach(conn, outbox);
        }
        ToHolder::Ack { through } => shared.lock().log.ack(through),
        ToHolder::Snapshot => {
            let copy = shared.lock().emulator.copy();
            outbox.send(FromHolder::Screen(capture(copy).await));
        }
        ToHolder::Subscribe => {
            let mut state = shared.lock();
            feed.start(state.emulator.copy());
            state.subscribers.insert(conn, feed.clone());
        }
        ToHolder::Input { bytes } => {
            let _ = shared.input.send(bytes);
        }
        ToHolder::Paste { text } => {
            let modes = shared.lock().emulator.input_modes();
            let _ = shared.input.send(encode_paste(&text, modes));
        }
        ToHolder::Resize(size) => shared.lock().resize(size),
        ToHolder::Kill => {
            let shared = shared.clone();
            tokio::spawn(async move { shared.stop_agent().await });
        }
        ToHolder::GuardHeld { id } => shared.lock().guards.hold(id),
        ToHolder::GuardAnswer { id, answer } => shared.lock().settle_guard(id, answer),
        ToHolder::Shutdown => {
            shared.stop_agent().await;
            shared.shutdown.notify_one();
            return false;
        }
        ToHolder::Hook { payload } => shared.lock().record(HolderEvent::Hook {
            payload,
            guard: None,
        }),
        ToHolder::Tap { payload } => shared.lock().record(HolderEvent::Tap { payload }),
        ToHolder::Guard { payload } => {
            let answer = shared.open_guard(payload).await.unwrap_or(GuardAnswer::Ask);
            outbox.send(FromHolder::GuardAnswer { answer });
        }
    }
    true
}
