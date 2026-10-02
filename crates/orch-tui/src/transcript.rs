use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use orch_core::SessionId;
use orch_protocol::TranscriptEntry;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use unicode_width::UnicodeWidthChar;

const TRIMMED_LINES: usize = 5;
const CALL: &str = "● ";
const RESULT: &str = "  ⎿ ";
const RESULT_MORE: &str = "    ";
const TAB_STOP: usize = 8;

pub(crate) struct Transcript {
    pub session: SessionId,
    pub subagent: String,
    entries: Vec<TranscriptEntry>,
    calls: HashSet<String>,
    results: HashMap<String, usize>,
    full: bool,
    top: Option<usize>,
    rows: RefCell<Option<(u16, Rc<[Line<'static>]>)>>,
}

struct Piece {
    prefix: &'static str,
    text: String,
    style: Style,
}

impl Piece {
    fn new(prefix: &'static str, text: impl Into<String>, style: Style) -> Self {
        Self {
            prefix,
            text: text.into(),
            style,
        }
    }
}

impl Transcript {
    pub fn new(session: SessionId, subagent: String) -> Self {
        Self {
            session,
            subagent,
            entries: Vec::new(),
            calls: HashSet::new(),
            results: HashMap::new(),
            full: false,
            top: None,
            rows: RefCell::new(None),
        }
    }

    pub fn is_of(&self, session: &SessionId, subagent: &str) -> bool {
        &self.session == session && self.subagent == subagent
    }

    pub fn receive(&mut self, entries: Vec<TranscriptEntry>, replace: bool) {
        if replace {
            self.entries.clear();
            self.calls.clear();
            self.results.clear();
        }
        for entry in entries {
            match &entry {
                TranscriptEntry::ToolCall { id, .. } => {
                    self.calls.insert(id.clone());
                }
                TranscriptEntry::ToolResult { id, .. } => {
                    self.results.entry(id.clone()).or_insert(self.entries.len());
                }
                _ => {}
            }
            self.entries.push(entry);
        }
        self.rows.take();
    }

    pub fn toggle_full(&mut self) {
        self.full = !self.full;
        self.rows.take();
    }

    pub fn top(&self, rows: usize, height: usize) -> usize {
        let last = rows.saturating_sub(height);
        self.top.map_or(last, |top| top.min(last))
    }

    pub fn scroll_up(&mut self, lines: isize, rows: usize, height: usize) {
        let last = rows.saturating_sub(height);
        let top = self.top(rows, height).saturating_add_signed(-lines);
        self.top = (top < last).then_some(top);
    }

    pub fn scroll_to_start(&mut self) {
        self.top = Some(0);
    }

    pub fn follow(&mut self) {
        self.top = None;
    }

    pub fn rows(&self, width: u16) -> Rc<[Line<'static>]> {
        if let Some((cached, rows)) = &*self.rows.borrow()
            && *cached == width
        {
            return rows.clone();
        }
        let columns = usize::from(width.max(1));
        let rows: Rc<[Line<'static>]> = self
            .pieces()
            .into_iter()
            .flat_map(|piece| {
                wrap(piece.prefix, &clean(&piece.text), columns)
                    .into_iter()
                    .map(move |row| Line::styled(row, piece.style))
            })
            .collect();
        *self.rows.borrow_mut() = Some((width, rows.clone()));
        rows
    }

    fn pieces(&self) -> Vec<Piece> {
        let mut pieces = Vec::new();
        for entry in &self.entries {
            let start = pieces.len();
            match entry {
                TranscriptEntry::Prompt { text } => {
                    pieces.extend(text_pieces(text, Style::new().fg(Color::Cyan)));
                }
                TranscriptEntry::Text { text } => pieces.extend(text_pieces(text, Style::new())),
                TranscriptEntry::ToolCall { id, tool, argument } => {
                    let call = match argument {
                        Some(argument) => format!("{tool} {argument}"),
                        None => tool.clone(),
                    };
                    let bold = Style::new().add_modifier(Modifier::BOLD);
                    pieces.push(Piece::new(CALL, call, bold));
                    if let Some((text, error)) = self.result_of(id) {
                        pieces.extend(self.result_pieces(text, error));
                    }
                }
                TranscriptEntry::ToolResult { id, text, error } => {
                    if !self.calls.contains(id) {
                        pieces.extend(self.result_pieces(text, *error));
                    }
                }
            }
            if pieces.len() > start {
                pieces.push(Piece::new("", "", Style::new()));
            }
        }
        pieces.pop();
        pieces
    }

    fn result_of(&self, call: &str) -> Option<(&str, bool)> {
        match self.entries.get(*self.results.get(call)?)? {
            TranscriptEntry::ToolResult { text, error, .. } => Some((text.as_str(), *error)),
            _ => None,
        }
    }

    fn result_pieces(&self, text: &str, error: bool) -> Vec<Piece> {
        let style = match error {
            true => Style::new().fg(Color::Red),
            false => Style::new().fg(Color::DarkGray),
        };
        let mut texts: Vec<String> = text.lines().map(String::from).collect();
        if texts.is_empty() {
            texts.push(String::new());
        }
        if error {
            texts[0] = format!("error: {}", texts[0]);
        }
        let hidden = match self.full {
            true => 0,
            false => texts.len().saturating_sub(TRIMMED_LINES),
        };
        texts.truncate(texts.len() - hidden);
        let mut pieces: Vec<_> = texts
            .into_iter()
            .enumerate()
            .map(|(at, text)| {
                let prefix = if at == 0 { RESULT } else { RESULT_MORE };
                Piece::new(prefix, text, style)
            })
            .collect();
        if hidden > 0 {
            let more = format!("… {hidden} more lines");
            pieces.push(Piece::new(
                RESULT_MORE,
                more,
                Style::new().fg(Color::DarkGray),
            ));
        }
        pieces
    }
}

fn text_pieces(text: &str, style: Style) -> Vec<Piece> {
    text.lines()
        .map(|line| Piece::new("", line, style))
        .collect()
}

fn clean(text: &str) -> String {
    let mut clean = String::with_capacity(text.len());
    let mut column = 0;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => skip_escape(&mut chars),
            '\t' => {
                let spaces = TAB_STOP - column % TAB_STOP;
                clean.extend(std::iter::repeat_n(' ', spaces));
                column += spaces;
            }
            c if c.is_control() => {}
            c => {
                clean.push(c);
                column += c.width().unwrap_or(0);
            }
        }
    }
    clean
}

fn skip_escape(chars: &mut std::iter::Peekable<std::str::Chars>) {
    match chars.next() {
        Some('[') => {
            for c in chars.by_ref() {
                if ('\x40'..='\x7e').contains(&c) {
                    break;
                }
            }
        }
        Some(']') => {
            while let Some(c) = chars.next() {
                if c == '\x07' || (c == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                    break;
                }
            }
        }
        _ => {}
    }
}

fn wrap(prefix: &str, text: &str, width: usize) -> Vec<String> {
    let indent = " ".repeat(prefix.chars().count());
    let mut rows = Vec::new();
    let mut row = prefix.to_string();
    let mut used = indent.len();
    for (at, c) in text.chars().enumerate() {
        let wide = c.width().unwrap_or(0);
        if used + wide > width && at > 0 {
            rows.push(std::mem::replace(&mut row, indent.clone()));
            used = indent.len();
        }
        row.push(c);
        used += wide;
    }
    rows.push(row);
    rows
}
