use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use orch_core::SessionId;
use orch_protocol::TranscriptEntry;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

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
    prefix: (&'static str, Style),
    spans: Vec<(String, Style)>,
}

impl Piece {
    fn plain(prefix: &'static str, text: impl Into<String>, style: Style) -> Self {
        Self {
            prefix: (prefix, style),
            spans: vec![(text.into(), style)],
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
            .iter()
            .flat_map(|piece| wrap(piece, columns))
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
                    let style = Style::new().fg(Color::Cyan);
                    pieces.extend(text.lines().map(|line| Piece::plain("", line, style)));
                }
                TranscriptEntry::Text { text } => {
                    pieces.extend(text.lines().map(|line| Piece {
                        prefix: ("", Style::new()),
                        spans: markdown(line),
                    }));
                }
                TranscriptEntry::ToolCall { id, tool, argument } => {
                    let result = self.result_of(id);
                    pieces.push(call_piece(tool, argument.as_deref(), result));
                    if let Some((text, error)) = result {
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
                pieces.push(Piece::plain("", "", Style::new()));
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
        if !self.full {
            texts = vec![match texts.len() {
                0 => "(no output)".into(),
                1 => texts.remove(0),
                _ if error => texts.remove(0),
                lines => format!("{lines} lines"),
            }];
        }
        if texts.is_empty() {
            texts.push(String::new());
        }
        if error {
            texts[0] = format!("error: {}", texts[0]);
        }
        texts
            .into_iter()
            .enumerate()
            .map(|(at, text)| {
                let prefix = if at == 0 { RESULT } else { RESULT_MORE };
                Piece::plain(prefix, text, style)
            })
            .collect()
    }
}

fn call_piece(tool: &str, argument: Option<&str>, result: Option<(&str, bool)>) -> Piece {
    let bullet = match result {
        None => Color::DarkGray,
        Some((_, true)) => Color::Red,
        Some((_, false)) => Color::Green,
    };
    let mut spans = vec![(tool.to_string(), Style::new().add_modifier(Modifier::BOLD))];
    if let Some(argument) = argument {
        let mut lines = argument.lines();
        let first = lines.next().unwrap_or_default();
        let more = if lines.next().is_some() { " …" } else { "" };
        spans.push((format!("({first}{more})"), Style::new()));
    }
    Piece {
        prefix: (CALL, Style::new().fg(bullet)),
        spans,
    }
}

fn markdown(line: &str) -> Vec<(String, Style)> {
    let code = Style::new().fg(Color::Magenta);
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let mut spans = Vec::new();
    let mut rest = line;
    loop {
        let next = [("`", code), ("**", bold)]
            .into_iter()
            .filter_map(|(marker, style)| {
                let open = rest.find(marker)?;
                let inner = &rest[open + marker.len()..];
                let close = inner.find(marker).filter(|close| *close > 0)?;
                Some((open, marker, style, close))
            })
            .min_by_key(|(open, ..)| *open);
        let Some((open, marker, style, close)) = next else {
            break;
        };
        spans.push((rest[..open].to_string(), Style::new()));
        let inner = &rest[open + marker.len()..];
        spans.push((inner[..close].to_string(), style));
        rest = &inner[close + marker.len()..];
    }
    spans.push((rest.to_string(), Style::new()));
    spans.retain(|(text, _)| !text.is_empty());
    spans
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

fn wrap(piece: &Piece, width: usize) -> Vec<Line<'static>> {
    let (prefix, prefix_style) = piece.prefix;
    let indent = " ".repeat(prefix.chars().count());
    let mut rows = Vec::new();
    let mut row = vec![Span::styled(prefix, prefix_style)];
    let mut used = indent.len();
    let mut first = true;
    for (text, style) in &piece.spans {
        for c in clean(text).chars() {
            let wide = c.width().unwrap_or(0);
            if used + wide > width && !first {
                let next = vec![Span::raw(indent.clone())];
                rows.push(Line::from(std::mem::replace(&mut row, next)));
                used = indent.len();
            }
            first = false;
            push_char(&mut row, c, *style);
            used += wide;
        }
    }
    rows.push(Line::from(row));
    rows
}

fn push_char(row: &mut Vec<Span<'static>>, c: char, style: Style) {
    match row.last_mut() {
        Some(last) if last.style == style => last.content.to_mut().push(c),
        _ => row.push(Span::styled(c.to_string(), style)),
    }
}
