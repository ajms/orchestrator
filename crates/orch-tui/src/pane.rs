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

    pub fn scrollback(&self) -> usize {
        self.parser.screen().scrollback()
    }

    pub fn cursor_line(&self) -> (i64, u16) {
        let (row, col) = self.parser.screen().cursor_position();
        let line = i64::from(row);
        match self.row_of(line) < i64::from(self.rows()) {
            true => (line, col),
            false => (self.line_of(0), 0),
        }
    }

    pub fn line_of(&self, row: u16) -> i64 {
        i64::from(row) - self.scrollback() as i64
    }

    pub fn row_of(&self, line: i64) -> i64 {
        line + self.scrollback() as i64
    }

    pub fn reveal(&mut self, line: i64) -> i64 {
        let last_row = i64::from(self.rows().saturating_sub(1));
        let row = self.row_of(line);
        if row < 0 {
            self.scroll_by(-row as isize);
        } else if row > last_row {
            self.scroll_by(-((row - last_row) as isize));
        }
        let row = self.row_of(line).clamp(0, last_row);
        self.line_of(row as u16)
    }

    pub fn text_of_lines(&mut self, start: (i64, u16), end: (i64, u16)) -> String {
        let kept = self.scrollback();
        let cols = self.parser.screen().size().1;
        let mut text = Vec::new();
        for line in start.0..=end.0 {
            self.set_scroll(usize::try_from(-line).unwrap_or(0));
            let row = self.row_of(line).max(0) as u16;
            let from = if line == start.0 { start.1 } else { 0 };
            let to = if line == end.0 { end.1 } else { cols };
            let contents = self.parser.screen().contents_between(row, from, row, to);
            text.push(contents.trim_end().to_string());
        }
        self.set_scroll(kept);
        text.join("\n")
    }
}
