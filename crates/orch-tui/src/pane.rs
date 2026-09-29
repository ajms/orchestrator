use orch_core::SessionId;
use orch_protocol::{ScreenSnapshot, Size};
use orch_term::keys::InputModes;

use crate::event::PaneId;

const SCROLLBACK: usize = 10_000;

pub struct PaneMirror {
    pub id: PaneId,
    pub session: SessionId,
    pub holder_pid: Option<u32>,
    parser: vt100::Parser,
    pub closed: Option<String>,
}

impl PaneMirror {
    pub fn new(id: PaneId, session: SessionId, holder_pid: Option<u32>, size: Size) -> Self {
        Self {
            id,
            session,
            holder_pid,
            parser: vt100::Parser::new(size.rows, size.cols, SCROLLBACK),
            closed: None,
        }
    }

    pub fn restore(&mut self, snapshot: &ScreenSnapshot) {
        self.parser = snapshot.restore(SCROLLBACK);
        self.closed = None;
    }

    pub fn output(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    pub fn resized(&mut self, size: Size) {
        self.parser.screen_mut().set_size(size.rows, size.cols);
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    pub fn input_modes(&self) -> InputModes {
        let screen = self.parser.screen();
        InputModes {
            application_cursor: screen.application_cursor(),
            bracketed_paste: screen.bracketed_paste(),
        }
    }
}

impl PaneMirror {
    pub fn scroll_by(&mut self, lines: isize) {
        let current = self.parser.screen().scrollback();
        self.set_scroll(current.saturating_add_signed(lines));
    }

    pub fn set_scroll(&mut self, offset: usize) {
        self.parser.screen_mut().set_scrollback(offset);
    }

    pub fn rows(&self) -> u16 {
        self.parser.screen().size().0
    }

    pub fn cursor(&self) -> (u16, u16) {
        self.parser.screen().cursor_position()
    }

    pub fn text_between(&self, start: (u16, u16), end: (u16, u16)) -> String {
        self.parser
            .screen()
            .contents_between(start.0, start.1, end.0, end.1)
    }
}
