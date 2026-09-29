use std::io::{self, Stdout, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc as std_mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use orch_protocol::{ConnectError, Size, daemon_socket};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::Instant;

use crate::config::TuiConfig;
use crate::event::{EditorError, Effect, Event};
use crate::git::load_review;
use crate::link::{DaemonLink, Tui};
use crate::socket::SocketLink;

const INPUT_POLL: Duration = Duration::from_millis(50);
const PAUSE_TIMEOUT: Duration = Duration::from_secs(1);
const AUTO_SCROLL_TICK: Duration = Duration::from_millis(50);

pub struct Options {
    pub runtime_dir: PathBuf,
    pub orch_program: PathBuf,
    pub config: TuiConfig,
}

pub async fn run(options: Options) -> io::Result<()> {
    let (events, mut inbox) = unbounded_channel();
    let mut screen = Screen::enter()?;
    let mut input = InputThread::spawn(events.clone());
    let result = event_loop(&options, &mut screen, &input, &events, &mut inbox).await;
    input.stop();
    result
}

async fn event_loop(
    options: &Options,
    screen: &mut Screen,
    input: &InputThread,
    events: &UnboundedSender<Event>,
    inbox: &mut UnboundedReceiver<Event>,
) -> io::Result<()> {
    let mut tui = connect(options, events, false).await?;
    let mut tick_at = Instant::now();
    loop {
        screen.terminal.draw(|frame| tui.render(frame))?;
        let Some(event) = next_event(&tui, inbox, &mut tick_at).await else {
            return Ok(());
        };
        let mut effects = tui.handle(event);
        while let Ok(event) = inbox.try_recv() {
            effects.extend(tui.handle(event));
        }
        for effect in effects {
            match effect {
                Effect::Quit => return Ok(()),
                Effect::WriteTerminal(bytes) => screen.write(&bytes)?,
                Effect::CopyCommand {
                    program,
                    args,
                    text,
                } => {
                    let events = events.clone();
                    tokio::task::spawn_blocking(move || {
                        if let Err(err) = copy(&program, &args, &text) {
                            let _ = events.send(Event::Notice(format!("copy: {err}")));
                        }
                    });
                }
                Effect::EditText { text } => {
                    let result = screen
                        .suspend(input, move || edit(&text))
                        .await
                        .unwrap_or_else(|err| Err(EditorError::Io(err)));
                    let _ = events.send(Event::EditorClosed(result));
                }
                Effect::RunExternal { command, cwd, env } => {
                    let result = screen
                        .suspend(input, move || shell(&command, &cwd, &env))
                        .await
                        .and_then(|result| result);
                    if let Err(err) = result {
                        let _ = events.send(Event::Notice(format!("external Review: {err}")));
                    }
                }
                Effect::LoadReview {
                    session,
                    target,
                    purpose,
                } => {
                    let events = events.clone();
                    tokio::task::spawn_blocking(move || {
                        let result = load_review(&target);
                        let _ = events.send(Event::Review {
                            session,
                            purpose,
                            result,
                        });
                    });
                }
                Effect::RestartDaemon => {
                    drop(tui);
                    discard_stale(inbox, events);
                    tui = connect(options, events, true).await?;
                }
            }
        }
    }
}

async fn next_event<L: DaemonLink>(
    tui: &Tui<L>,
    inbox: &mut UnboundedReceiver<Event>,
    tick_at: &mut Instant,
) -> Option<Event> {
    if !tui.auto_scrolling() {
        *tick_at = Instant::now() + AUTO_SCROLL_TICK;
        return inbox.recv().await;
    }
    if Instant::now() < *tick_at
        && let Ok(event) = tokio::time::timeout_at(*tick_at, inbox.recv()).await
    {
        return event;
    }
    *tick_at = Instant::now() + AUTO_SCROLL_TICK;
    Some(Event::Tick)
}

fn discard_stale(inbox: &mut UnboundedReceiver<Event>, events: &UnboundedSender<Event>) {
    let mut kept = Vec::new();
    while let Ok(event) = inbox.try_recv() {
        if !matches!(
            event,
            Event::Daemon(_) | Event::Pane { .. } | Event::Disconnected { .. }
        ) {
            kept.push(event);
        }
    }
    for event in kept {
        let _ = events.send(event);
    }
}

async fn connect(
    options: &Options,
    events: &UnboundedSender<Event>,
    restart: bool,
) -> io::Result<Tui<SocketLink>> {
    let socket = daemon_socket(&options.runtime_dir);
    let (cols, rows) = crossterm::terminal::size()?;
    let size = Size { rows, cols };
    let (runtime_dir, program) = (&options.runtime_dir, &options.orch_program);
    let ensured = match restart {
        true => orch_protocol::restart_running_daemon(runtime_dir, program).await,
        false => orch_protocol::connect_or_spawn_daemon(runtime_dir, program).await,
    };
    let link = match ensured {
        Ok(_) => SocketLink::connect(&socket, events.clone()).await,
        Err(err) => Err(err),
    };
    let link = match link {
        Ok(link) => link,
        Err(ConnectError::VersionMismatch { message, .. }) => {
            let _ = events.send(Event::VersionMismatch { message });
            SocketLink::offline(&socket, events.clone())
        }
        Err(ConnectError::Io(err)) => return Err(err),
    };
    Ok(Tui::new(options.config.clone(), link, size))
}

fn edit(text: &str) -> Result<String, EditorError> {
    let file = tempfile::Builder::new()
        .prefix("orch-")
        .suffix(".md")
        .tempfile()?;
    std::fs::write(file.path(), text)?;
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into());
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(file.path())
        .status()?;
    if !status.success() {
        return Err(EditorError::Exited { editor, status });
    }
    Ok(std::fs::read_to_string(file.path())?)
}

fn copy(program: &str, args: &[String], text: &str) -> io::Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| io::Error::new(err.kind(), format!("{program}: {err}")))?;
    let written = child
        .stdin
        .take()
        .map_or(Ok(()), |mut stdin| stdin.write_all(text.as_bytes()));
    let status = child.wait()?;
    written?;
    match status.success() {
        true => Ok(()),
        false => Err(io::Error::other(format!("{program} exited with {status}"))),
    }
}

fn shell(command: &str, cwd: &Path, env: &[(String, String)]) -> io::Result<()> {
    let status = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .envs(env.iter().map(|(key, value)| (key, value)))
        .status()?;
    match status.success() {
        true => Ok(()),
        false => Err(io::Error::other(format!(
            "`{command}` exited with {status}"
        ))),
    }
}

struct Screen {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Screen {
    fn enter() -> io::Result<Self> {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = leave();
            previous(info);
        }));
        start()?;
        Ok(Self {
            terminal: Terminal::new(CrosstermBackend::new(io::stdout()))?,
        })
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut out = io::stdout();
        out.write_all(bytes)?;
        out.flush()
    }

    async fn suspend<T: Send + 'static>(
        &mut self,
        input: &InputThread,
        run: impl FnOnce() -> T + Send + 'static,
    ) -> io::Result<T> {
        input.pause();
        let _ = leave();
        let result = tokio::task::spawn_blocking(run)
            .await
            .map_err(io::Error::other);
        let restarted = start();
        let _ = self.terminal.clear();
        input.resume();
        restarted?;
        result
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = leave();
    }
}

fn start() -> io::Result<()> {
    enable_raw_mode()?;
    let entered = execute!(
        io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableFocusChange,
        EnableBracketedPaste
    );
    if entered.is_err() {
        let _ = leave();
    }
    entered
}

fn leave() -> io::Result<()> {
    let left = execute!(
        io::stdout(),
        DisableBracketedPaste,
        DisableFocusChange,
        DisableMouseCapture,
        LeaveAlternateScreen
    );
    disable_raw_mode()?;
    left
}

enum Control {
    Pause,
    Resume,
    Stop,
}

struct InputThread {
    control: std_mpsc::Sender<Control>,
    parked: std_mpsc::Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

impl InputThread {
    fn spawn(events: UnboundedSender<Event>) -> Self {
        let (control, orders) = std_mpsc::channel();
        let (acknowledge, parked) = std_mpsc::channel();
        let thread = std::thread::spawn(move || read_input(&events, &orders, &acknowledge));
        Self {
            control,
            parked,
            thread: Some(thread),
        }
    }

    fn pause(&self) {
        if self.control.send(Control::Pause).is_ok() {
            let _ = self.parked.recv_timeout(PAUSE_TIMEOUT);
        }
    }

    fn resume(&self) {
        let _ = self.control.send(Control::Resume);
    }

    fn stop(&mut self) {
        let _ = self.control.send(Control::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn read_input(
    events: &UnboundedSender<Event>,
    orders: &std_mpsc::Receiver<Control>,
    acknowledge: &std_mpsc::Sender<()>,
) {
    loop {
        match orders.try_recv() {
            Ok(Control::Stop) | Err(std_mpsc::TryRecvError::Disconnected) => return,
            Ok(Control::Pause) => {
                let _ = acknowledge.send(());
                loop {
                    match orders.recv() {
                        Ok(Control::Resume) => break,
                        Ok(Control::Stop) | Err(_) => return,
                        Ok(Control::Pause) => {}
                    }
                }
            }
            Ok(Control::Resume) | Err(std_mpsc::TryRecvError::Empty) => {}
        }
        let event = match crossterm::event::poll(INPUT_POLL) {
            Ok(true) => crossterm::event::read(),
            Ok(false) => continue,
            Err(err) => Err(err),
        };
        let event = match event {
            Ok(event) => Event::Terminal(event),
            Err(err) => Event::Notice(format!("terminal input failed: {err}")),
        };
        if events.send(event).is_err() {
            return;
        }
    }
}
