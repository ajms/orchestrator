use std::collections::VecDeque;
use std::path::PathBuf;

use crossterm::event::{Event as TermEvent, KeyCode, KeyEvent, KeyModifiers};
use orch_core::SessionId;
use orch_protocol::{
    AgentStateView, FlagsView, FromDaemon, PhaseView, Reply, Request, RequestError, ScreenSnapshot,
    SessionView, Size,
};
use orch_tui::{DaemonLink, Effect, Event, PaneId, RequestId, Tui, TuiConfig};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

pub const WIDTH: u16 = 120;
pub const HEIGHT: u16 = 30;
pub const SIDEBAR: u16 = 40;

#[derive(Default)]
pub struct FakeDaemon {
    pub requests: Vec<(RequestId, Request)>,
    pub opened: Vec<(SessionId, Size)>,
    pub pane_ids: Vec<(PaneId, SessionId)>,
    pub closed: usize,
    pub input: Vec<u8>,
    pub pastes: Vec<String>,
    pub resizes: Vec<Size>,
    pub screens: Vec<(SessionId, ScreenSnapshot)>,
    replies: VecDeque<Result<Reply, RequestError>>,
    outbox: VecDeque<Event>,
}

impl FakeDaemon {
    pub fn script_reply(&mut self, reply: Result<Reply, RequestError>) {
        self.replies.push_back(reply);
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests
            .iter()
            .map(|(_, request)| request.clone())
            .collect()
    }

    pub fn views(&self) -> Vec<(Option<String>, bool)> {
        self.requests()
            .into_iter()
            .filter_map(|request| match request {
                Request::View { session, focused } => {
                    Some((session.map(|id| id.as_str().to_string()), focused))
                }
                _ => None,
            })
            .collect()
    }

    pub fn last_view(&self) -> Option<(Option<String>, bool)> {
        self.views().pop()
    }

    pub fn pane_id(&self, session: &str) -> PaneId {
        self.pane_ids
            .iter()
            .rev()
            .find(|(_, id)| id.as_str() == session)
            .map(|(pane, _)| *pane)
            .unwrap_or_else(|| panic!("no pane opened for {session}"))
    }

    pub fn open_panes(&self) -> Vec<String> {
        self.opened
            .iter()
            .map(|(id, _)| id.as_str().to_string())
            .collect()
    }
}

impl DaemonLink for FakeDaemon {
    fn request(&mut self, id: RequestId, request: Request) {
        self.requests.push((id, request));
        let result = self.replies.pop_front().unwrap_or(Ok(Reply::Done));
        self.outbox
            .push_back(Event::Daemon(FromDaemon::Response { id: id.0, result }));
    }

    fn open_pane(&mut self, pane: PaneId, session: &SessionId, size: Size) {
        self.opened.push((session.clone(), size));
        self.pane_ids.push((pane, session.clone()));
        if let Some((_, screen)) = self.screens.iter().find(|(id, _)| id == session) {
            self.outbox.push_back(Event::Pane {
                pane,
                message: FromDaemon::Screen(screen.clone()),
            });
        }
    }

    fn close_pane(&mut self) {
        self.closed += 1;
    }

    fn input(&mut self, bytes: Vec<u8>) {
        self.input.extend(bytes);
    }

    fn paste(&mut self, text: String) {
        self.pastes.push(text);
    }

    fn resize_pane(&mut self, size: Size) {
        self.resizes.push(size);
    }
}

pub struct Harness {
    pub tui: Tui<FakeDaemon>,
    pub terminal: Terminal<TestBackend>,
    pub effects: Vec<Effect>,
}

impl Harness {
    pub fn new() -> Self {
        Self::with_config(TuiConfig::default())
    }

    pub fn with_config(config: TuiConfig) -> Self {
        Self::build(config, FakeDaemon::default())
    }

    pub fn with_screens(screens: Vec<(SessionId, ScreenSnapshot)>) -> Self {
        let daemon = FakeDaemon {
            screens,
            ..FakeDaemon::default()
        };
        Self::build(TuiConfig::default(), daemon)
    }

    pub fn unfocused() -> Self {
        let size = Size {
            rows: HEIGHT,
            cols: WIDTH,
        };
        Self {
            tui: Tui::new(TuiConfig::default(), FakeDaemon::default(), size),
            terminal: Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap(),
            effects: Vec::new(),
        }
    }

    fn build(config: TuiConfig, daemon: FakeDaemon) -> Self {
        let size = Size {
            rows: HEIGHT,
            cols: WIDTH,
        };
        let mut harness = Self {
            tui: Tui::new(config, daemon, size),
            terminal: Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap(),
            effects: Vec::new(),
        };
        harness.send(Event::Terminal(TermEvent::FocusGained));
        harness
    }

    pub fn daemon(&mut self) -> &mut FakeDaemon {
        self.tui.link_mut()
    }

    pub fn send(&mut self, event: Event) {
        let effects = self.tui.handle(event);
        self.effects.extend(effects);
        while let Some(event) = self.tui.link_mut().outbox.pop_front() {
            let effects = self.tui.handle(event);
            self.effects.extend(effects);
        }
    }

    pub fn sessions(&mut self, sessions: Vec<SessionView>) {
        self.send(Event::Daemon(FromDaemon::Sessions { sessions }));
    }

    pub fn changed(&mut self, session: SessionView) {
        self.send(Event::Daemon(FromDaemon::SessionChanged {
            session: Box::new(session),
        }));
    }

    pub fn pane(&mut self, session: &str, message: FromDaemon) {
        let pane = self.daemon().pane_id(session);
        self.send(Event::Pane { pane, message });
    }

    pub fn press(&mut self, code: KeyCode) {
        self.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    pub fn ctrl(&mut self, c: char) {
        self.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    }

    pub fn key(&mut self, key: KeyEvent) {
        self.send(Event::Terminal(TermEvent::Key(key)));
    }

    pub fn keys(&mut self, text: &str) {
        for c in text.chars() {
            let code = match c {
                '\n' => KeyCode::Enter,
                c => KeyCode::Char(c),
            };
            self.press(code);
        }
    }

    pub fn command(&mut self, line: &str) {
        self.keys(&format!(":{line}\n"));
    }

    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    pub fn draw(&mut self) {
        let tui = &self.tui;
        self.terminal.draw(|frame| tui.render(frame)).unwrap();
    }

    pub fn screen(&mut self) -> String {
        self.lines().join("\n")
    }

    pub fn lines(&mut self) -> Vec<String> {
        self.draw();
        let buffer = self.terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect()
    }

    pub fn sidebar_lines(&mut self) -> Vec<String> {
        self.lines()
            .into_iter()
            .map(|line| line.chars().take(usize::from(SIDEBAR)).collect())
            .collect()
    }

    pub fn sidebar_line_with(&mut self, needle: &str) -> String {
        let lines = self.sidebar_lines();
        lines
            .into_iter()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("no sidebar line contains {needle:?} in\n{}", self.screen()))
    }

    pub fn sidebar_background_of(&mut self, needle: &str) -> Color {
        let lines = self.sidebar_lines();
        let (y, line) = lines
            .iter()
            .enumerate()
            .find(|(_, line)| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} not in the sidebar"));
        let column = line[..line.find(needle).unwrap()].chars().count() as u16;
        self.terminal.backend().buffer()[(column, y as u16)].bg
    }

    pub fn line_with(&mut self, needle: &str) -> String {
        let lines = self.lines();
        lines
            .into_iter()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("no line contains {needle:?} in\n{}", self.screen()))
    }

    pub fn colour_of(&mut self, needle: &str) -> Color {
        self.draw();
        let buffer = self.terminal.backend().buffer().clone();
        for y in 0..buffer.area.height {
            let line: Vec<String> = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect();
            let text = line.concat();
            if let Some(byte) = text.find(needle) {
                let column = text[..byte].chars().count() as u16;
                return buffer[(column, y)].fg;
            }
        }
        panic!("{needle:?} not on screen:\n{}", self.screen());
    }

    pub fn background_of(&mut self, needle: &str) -> Color {
        self.draw();
        let buffer = self.terminal.backend().buffer().clone();
        for y in 0..buffer.area.height {
            let text: String = (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect();
            if let Some(byte) = text.find(needle) {
                let column = text[..byte].chars().count() as u16;
                return buffer[(column, y)].bg;
            }
        }
        panic!("{needle:?} not on screen:\n{}", self.screen());
    }
}

pub fn id(text: &str) -> SessionId {
    SessionId(text.to_string())
}

pub fn session(repo: &str, slug: &str) -> SessionView {
    let repo = PathBuf::from(format!("/home/me/{repo}"));
    SessionView {
        id: id(slug),
        worktree: repo.join(".orchestrator/worktrees").join(slug),
        repo,
        slug: slug.into(),
        branch: format!("orch/{slug}"),
        base: "main".into(),
        phase: PhaseView::Active,
        agent: Some(AgentStateView::Idle),
        flags: FlagsView::default(),
        preset: "inherit".into(),
        mode: None,
        conversation: None,
        port_base: None,
        port_size: None,
        setup_output: None,
        error: None,
        holder_pid: None,
        guards_enabled: true,
        guard_prompts: Vec::new(),
        context_used_percent: None,
        cost_usd: None,
        subagents: Vec::new(),
    }
}

pub fn with_agent(mut view: SessionView, state: AgentStateView) -> SessionView {
    view.agent = Some(state);
    view
}

pub fn in_phase(mut view: SessionView, phase: PhaseView) -> SessionView {
    view.phase = phase;
    view.agent = None;
    view
}

pub fn screen_of(text: &str, size: Size) -> ScreenSnapshot {
    let mut parser = vt100::Parser::new(size.rows, size.cols, 0);
    parser.process(text.as_bytes());
    let screen = parser.screen();
    ScreenSnapshot {
        size,
        scrollback: Vec::new(),
        screen: screen.contents_formatted(),
        alternate: None,
        input_modes: screen.input_mode_formatted(),
    }
}
