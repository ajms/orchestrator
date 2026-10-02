use std::cell::Cell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crossterm::event::{
    Event as TermEvent, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use orch_core::SessionId;
use orch_protocol::{
    AgentStateView, CreateSession, FlagsView, FromDaemon, PhaseView, Reply, RepoSettings, Request,
    RequestError, ScreenSnapshot, SessionView, Size, SubagentView,
};
use orch_tui::{DaemonLink, Effect, Event, PaneId, RequestId, Tui, TuiConfig};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

pub const WIDTH: u16 = 120;
pub const HEIGHT: u16 = 30;
pub const SIDEBAR: u16 = 40;
pub const PANE_LEFT: u16 = SIDEBAR + 1;
pub const PANE_TOP: u16 = 1;

#[derive(Default)]
pub struct FakeDaemon {
    pub requests: Vec<(RequestId, Request)>,
    pub opened: Vec<(SessionId, Size)>,
    pub pane_ids: Vec<(PaneId, SessionId)>,
    pub closed: usize,
    pub input: Vec<u8>,
    pub input_by_session: Vec<(Option<String>, Vec<u8>)>,
    shown: Option<String>,
    pub pastes: Vec<String>,
    pub resizes: Vec<Size>,
    pub screens: Vec<(SessionId, ScreenSnapshot)>,
    pub repos: Option<Vec<PathBuf>>,
    pub settings: Vec<RepoSettings>,
    replies: VecDeque<Result<Reply, RequestError>>,
    outbox: VecDeque<Event>,
    pub hold: bool,
    held: Vec<Event>,
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

    pub fn input_to(&self, session: &str) -> String {
        let bytes: Vec<u8> = self
            .input_by_session
            .iter()
            .filter(|(shown, _)| shown.as_deref() == Some(session))
            .flat_map(|(_, bytes)| bytes.clone())
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
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
        let result = match &request {
            Request::Repos => Ok(self
                .repos
                .clone()
                .map_or(Reply::Done, |repos| Reply::Repos { repos })),
            Request::RepoSettings { repo } => Ok(self
                .settings
                .iter()
                .find(|settings| &settings.repo == repo)
                .cloned()
                .map_or(Reply::Done, Reply::RepoSettings)),
            _ => self.replies.pop_front().unwrap_or(Ok(Reply::Done)),
        };
        let held = self.hold && !matches!(request, Request::Repos | Request::RepoSettings { .. });
        self.requests.push((id, request));
        let response = Event::Daemon(FromDaemon::Response { id: id.0, result });
        match held {
            true => self.held.push(response),
            false => self.outbox.push_back(response),
        }
    }

    fn open_pane(&mut self, pane: PaneId, session: &SessionId, size: Size) {
        self.opened.push((session.clone(), size));
        self.shown = Some(session.as_str().to_string());
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
        self.shown = None;
    }

    fn input(&mut self, bytes: Vec<u8>) {
        self.input_by_session
            .push((self.shown.clone(), bytes.clone()));
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
    clock: Rc<Cell<Instant>>,
}

fn tui_with_clock(config: TuiConfig, daemon: FakeDaemon) -> (Tui<FakeDaemon>, Rc<Cell<Instant>>) {
    let size = Size {
        rows: HEIGHT,
        cols: WIDTH,
    };
    let clock = Rc::new(Cell::new(Instant::now()));
    let now = clock.clone();
    let tui = Tui::new(config, daemon, size).with_clock(move || now.get());
    (tui, clock)
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
        let (tui, clock) = tui_with_clock(TuiConfig::default(), FakeDaemon::default());
        Self {
            tui,
            terminal: Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap(),
            effects: Vec::new(),
            clock,
        }
    }

    fn build(config: TuiConfig, daemon: FakeDaemon) -> Self {
        let (tui, clock) = tui_with_clock(config, daemon);
        let mut harness = Self {
            tui,
            terminal: Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap(),
            effects: Vec::new(),
            clock,
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

    pub fn release_replies(&mut self) {
        let held = std::mem::take(&mut self.daemon().held);
        self.daemon().hold = false;
        for event in held {
            self.send(event);
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

    pub fn mouse(&mut self, kind: MouseEventKind, column: u16, row: u16, modifiers: KeyModifiers) {
        let event = MouseEvent {
            kind,
            column,
            row,
            modifiers,
        };
        self.send(Event::Terminal(TermEvent::Mouse(event)));
    }

    pub fn mouse_down(&mut self, column: u16, row: u16) {
        let kind = MouseEventKind::Down(MouseButton::Left);
        self.mouse(kind, column, row, KeyModifiers::NONE);
    }

    pub fn drag(&mut self, column: u16, row: u16) {
        let kind = MouseEventKind::Drag(MouseButton::Left);
        self.mouse(kind, column, row, KeyModifiers::NONE);
    }

    pub fn release(&mut self, column: u16, row: u16) {
        let kind = MouseEventKind::Up(MouseButton::Left);
        self.mouse(kind, column, row, KeyModifiers::NONE);
    }

    pub fn click(&mut self, column: u16, row: u16) {
        self.mouse_down(column, row);
        self.release(column, row);
    }

    pub fn wheel(&mut self, kind: MouseEventKind, column: u16, row: u16) {
        self.mouse(kind, column, row, KeyModifiers::NONE);
    }

    pub fn later(&mut self, millis: u64) {
        self.clock
            .set(self.clock.get() + Duration::from_millis(millis));
    }

    pub fn tick(&mut self) {
        self.send(Event::Tick);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.terminal.backend_mut().resize(cols, rows);
        self.send(Event::Terminal(TermEvent::Resize(cols, rows)));
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

    pub fn cursor(&mut self) -> Option<(u16, u16)> {
        self.draw();
        let backend = self.terminal.backend();
        let at = backend.cursor_position();
        backend.cursor_visible().then_some((at.x, at.y))
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

    pub fn sidebar_colour_of(&mut self, needle: &str) -> Color {
        self.sidebar_cell(needle).fg
    }

    pub fn sidebar_background_of(&mut self, needle: &str) -> Color {
        self.sidebar_cell(needle).bg
    }

    fn sidebar_cell(&mut self, needle: &str) -> ratatui::buffer::Cell {
        let lines = self.sidebar_lines();
        let (y, line) = lines
            .iter()
            .enumerate()
            .find(|(_, line)| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} not in the sidebar"));
        let column = line[..line.find(needle).unwrap()].chars().count() as u16;
        self.terminal.backend().buffer()[(column, y as u16)].clone()
    }

    pub fn pane_lines(&mut self) -> Vec<String> {
        self.lines()
            .into_iter()
            .map(|line| line.chars().skip(usize::from(PANE_LEFT)).collect())
            .collect()
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

pub fn sidebar_row_of(tui: &mut Harness, needle: &str) -> u16 {
    let lines = tui.sidebar_lines();
    lines
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not in the sidebar:\n{}", lines.join("\n")))
        as u16
}

pub fn click_row(tui: &mut Harness, needle: &str) {
    let row = sidebar_row_of(tui, needle);
    tui.click(5, row);
}

pub fn shown(tui: &mut Harness) -> Option<String> {
    tui.daemon().last_view().and_then(|(session, _)| session)
}

pub fn in_sidebar(tui: &mut Harness, needle: &str) -> bool {
    tui.sidebar_lines().iter().any(|line| line.contains(needle))
}

pub fn create_requests(tui: &mut Harness) -> Vec<CreateSession> {
    tui.daemon()
        .requests()
        .into_iter()
        .filter_map(|request| match request {
            Request::CreateSession(create) => Some(create),
            _ => None,
        })
        .collect()
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
        title: None,
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

pub fn subagent(id: &str, agent_type: &str, description: &str, done: bool) -> SubagentView {
    SubagentView {
        id: id.into(),
        agent_type: agent_type.into(),
        description: description.into(),
        tool_count: 3,
        done,
    }
}

pub fn titled(mut view: SessionView, title: &str) -> SessionView {
    view.title = Some(title.into());
    view
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

pub fn in_pane(col: u16, row: u16) -> (u16, u16) {
    (PANE_LEFT + col, PANE_TOP + row)
}

pub fn statusline(tui: &mut Harness) -> String {
    tui.lines().pop().unwrap()
}
