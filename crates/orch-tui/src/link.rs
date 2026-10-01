use std::time::{Duration, Instant};

use orch_core::SessionId;
use orch_protocol::{Request, Size};
use ratatui::Frame;

use crate::app::{App, Call, TuiConfig};
use crate::event::{Effect, Event, PaneId};
use crate::preparing::Preparing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestId(pub u64);

const AUTO_SCROLL_TICK: Duration = Duration::from_millis(50);
const PREPARING_TICK: Duration = Duration::from_secs(1);

pub trait DaemonLink {
    fn request(&mut self, id: RequestId, request: Request);
    fn open_pane(&mut self, pane: PaneId, session: &SessionId, size: Size);
    fn close_pane(&mut self);
    fn input(&mut self, bytes: Vec<u8>);
    fn paste(&mut self, text: String);
    fn resize_pane(&mut self, size: Size);
}

pub struct Tui<L> {
    app: App,
    link: L,
}

impl<L: DaemonLink> Tui<L> {
    pub fn new(config: TuiConfig, link: L, size: Size) -> Self {
        Self {
            app: App::new(config, size),
            link,
        }
    }

    pub fn with_clock(mut self, clock: impl Fn() -> Instant + 'static) -> Self {
        self.app.clock = Box::new(clock);
        self
    }

    pub fn tick_every(&self) -> Option<Duration> {
        if crate::mouse::auto_scrolling(&self.app) {
            return Some(AUTO_SCROLL_TICK);
        }
        self.app
            .preparing
            .iter()
            .any(Preparing::is_counting)
            .then_some(PREPARING_TICK)
    }

    pub fn link_mut(&mut self) -> &mut L {
        &mut self.link
    }

    pub fn handle(&mut self, event: Event) -> Vec<Effect> {
        let mut local = Vec::new();
        for call in self.app.update(event) {
            match call {
                Call::Request(id, request) => self.link.request(id, request),
                Call::OpenPane(pane, session, size) => self.link.open_pane(pane, &session, size),
                Call::ClosePane => self.link.close_pane(),
                Call::Input(bytes) => self.link.input(bytes),
                Call::Paste(text) => self.link.paste(text),
                Call::ResizePane(size) => self.link.resize_pane(size),
                Call::Local(effect) => local.push(effect),
            }
        }
        local
    }

    pub fn render(&self, frame: &mut Frame) {
        crate::render::draw(&self.app, frame);
    }
}
