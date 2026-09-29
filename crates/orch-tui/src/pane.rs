use orch_core::SessionId;
use orch_protocol::{ScreenSnapshot, Size};
use orch_term::keys::InputModes;
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

use crate::event::PaneId;
use crate::selection::{Grid, Row};

const SCROLLBACK: usize = 10_000;

pub struct PaneMirror {
    pub id: PaneId,
    pub session: SessionId,
    pub holder_pid: Option<u32>,
    parser: vt100::Parser,
    focus: FocusRequest,
    pub told_focused: bool,
    pub closed: Option<String>,
}

impl PaneMirror {
    pub fn new(id: PaneId, session: SessionId, holder_pid: Option<u32>, size: Size) -> Self {
        Self {
            id,
            session,
            holder_pid,
            parser: vt100::Parser::new(size.rows, size.cols, SCROLLBACK),
            focus: FocusRequest::default(),
            told_focused: false,
            closed: None,
        }
    }

    pub fn restore(&mut self, snapshot: &ScreenSnapshot) {
        self.parser = snapshot.restore(SCROLLBACK);
        self.focus = FocusRequest::default();
        self.focus.scan(&snapshot.input_modes);
        self.closed = None;
    }

    pub fn output(&mut self, bytes: &[u8]) -> Option<i64> {
        let before = self.history();
        let ends = (before == SCROLLBACK).then(|| self.history_ends());
        self.parser.process(bytes);
        self.focus.scan(bytes);
        let after = self.history();
        if after < SCROLLBACK {
            return Some(after as i64 - before as i64);
        }
        let unchanged = ends.is_some_and(|ends| ends == self.history_ends());
        unchanged.then_some(0)
    }

    fn history(&mut self) -> usize {
        self.with_scroll(usize::MAX, |mirror| mirror.scrollback())
    }

    fn history_ends(&mut self) -> (Row, Row) {
        (self.row(-(SCROLLBACK as i64)), self.row(-1))
    }

    fn with_scroll<T>(&mut self, offset: usize, read: impl FnOnce(&Self) -> T) -> T {
        let kept = self.scrollback();
        self.set_scroll(offset);
        let value = read(self);
        self.set_scroll(kept);
        value
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

    pub fn mouse_mode(&self) -> MouseProtocolMode {
        self.parser.screen().mouse_protocol_mode()
    }

    pub fn mouse_encoding(&self) -> MouseProtocolEncoding {
        self.parser.screen().mouse_protocol_encoding()
    }

    pub fn wants_focus(&self) -> bool {
        self.focus.reporting.requested
    }
}

#[derive(Default)]
struct FocusRequest {
    parser: vte::Parser,
    reporting: FocusReporting,
}

impl FocusRequest {
    fn scan(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.reporting, bytes);
    }
}

#[derive(Default)]
struct FocusReporting {
    requested: bool,
}

impl vte::Perform for FocusReporting {
    fn csi_dispatch(&mut self, params: &vte::Params, intermediates: &[u8], _: bool, action: char) {
        let on = match (intermediates, action) {
            (b"?", 'h') => true,
            (b"?", 'l') => false,
            _ => return,
        };
        if params.iter().any(|param| param == [1004]) {
            self.requested = on;
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
        let cols = self.parser.screen().size().1;
        let mut text = Vec::new();
        for line in start.0..=end.0 {
            let from = if line == start.0 { start.1 } else { 0 };
            let to = if line == end.0 { end.1 } else { cols };
            let contents = self.with_scroll(offset_of(line), |mirror| {
                let row = mirror.row_of(line).max(0) as u16;
                mirror.screen().contents_between(row, from, row, to)
            });
            text.push(contents.trim_end().to_string());
        }
        text.join("\n")
    }
}

impl Grid for PaneMirror {
    fn height(&self) -> u16 {
        self.rows()
    }

    fn width(&self) -> u16 {
        self.parser.screen().size().1
    }

    fn line_at(&self, row: u16) -> i64 {
        self.line_of(row)
    }

    fn scroll(&mut self, lines: isize) {
        self.scroll_by(lines);
    }

    fn row(&mut self, line: i64) -> Row {
        let row = self.with_scroll(offset_of(line), |mirror| {
            let row = mirror.row_of(line);
            (0..i64::from(mirror.rows()))
                .contains(&row)
                .then(|| mirror.cells_of(row as u16))
        });
        row.unwrap_or(Row {
            cells: Vec::new(),
            wrapped: false,
        })
    }
}

impl PaneMirror {
    fn cells_of(&self, row: u16) -> Row {
        let screen = self.parser.screen();
        let cells = (0..screen.size().1)
            .map(|col| match screen.cell(row, col) {
                Some(cell) if cell.is_wide_continuation() => String::new(),
                Some(cell) if cell.has_contents() => cell.contents().to_string(),
                _ => " ".to_string(),
            })
            .collect();
        Row {
            cells,
            wrapped: screen.row_wrapped(row),
        }
    }
}

fn offset_of(line: i64) -> usize {
    usize::try_from(-line).unwrap_or(0)
}
