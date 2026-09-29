use std::path::PathBuf;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use crate::selection::{Grid, Row, Selector};

const SCROLL_STEP: isize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    pub lines: Vec<String>,
    pub deleted: bool,
}

impl FileDiff {
    pub fn added(&self) -> usize {
        self.lines
            .iter()
            .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
            .count()
    }

    pub fn removed(&self) -> usize {
        self.lines
            .iter()
            .filter(|line| line.starts_with('-') && !line.starts_with("---"))
            .count()
    }
}

pub(crate) fn parse_unified_diff(diff: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    for line in diff.lines() {
        if let Some(header) = line.strip_prefix("diff --git ") {
            let path = header
                .split_once(" b/")
                .map_or(header, |(_, path)| path)
                .to_string();
            files.push(FileDiff {
                path,
                lines: Vec::new(),
                deleted: false,
            });
        } else if let Some(file) = files.last_mut() {
            let in_hunks = line.starts_with("@@") || !file.lines.is_empty();
            match in_hunks {
                true => file.lines.push(line.to_string()),
                false if line.starts_with("deleted file mode") => file.deleted = true,
                false => {}
            }
        }
    }
    files
}

pub(crate) enum ReviewAction {
    Stay,
    Close,
    CommandLine,
    Open(EditorTarget),
    Notice(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EditorTarget {
    pub file: PathBuf,
    pub line: Option<u32>,
}

pub(crate) struct ReviewView {
    pub base: String,
    pub worktree: PathBuf,
    pub files: Vec<FileDiff>,
    pub file: usize,
    pub scroll: u16,
    pub list_scroll: u16,
    pub selection: Selector,
}

impl ReviewView {
    pub fn new(base: String, worktree: PathBuf, files: Vec<FileDiff>) -> Self {
        Self {
            base,
            worktree,
            files,
            file: 0,
            scroll: 0,
            list_scroll: 0,
            selection: Selector::default(),
        }
    }

    pub fn key(&mut self, key: KeyEvent, body: Rect) -> ReviewAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let last = self.files.len().saturating_sub(1);
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return ReviewAction::Close,
            KeyCode::Char(':') => return ReviewAction::CommandLine,
            KeyCode::Char('o') => return self.open_first_hunk(),
            KeyCode::Char('d') if ctrl => self.scroll_diff(-SCROLL_STEP, body),
            KeyCode::Char('u') if ctrl => self.scroll_diff(SCROLL_STEP, body),
            KeyCode::Char('j') | KeyCode::Down => self.show_file((self.file + 1).min(last)),
            KeyCode::Char('k') | KeyCode::Up => self.show_file(self.file.saturating_sub(1)),
            _ => {}
        }
        ReviewAction::Stay
    }

    pub fn show_file(&mut self, file: usize) {
        if file != self.file {
            self.selection.clear();
        }
        self.file = file;
        self.scroll = 0;
    }

    pub fn file_at(&self, row: u16) -> Option<usize> {
        let at = usize::from(self.list_scroll) + usize::from(row);
        (at < self.files.len()).then_some(at)
    }

    pub fn scroll_list(&mut self, lines: isize, height: u16) {
        let most = self.files.len().saturating_sub(usize::from(height));
        self.list_scroll = scrolled_up(self.list_scroll, lines, most);
    }

    pub fn scroll_diff(&mut self, lines: isize, body: Rect) {
        self.diff_grid(body).grid.scroll(lines);
    }

    pub fn open_first_hunk(&self) -> ReviewAction {
        let Some(file) = self.files.get(self.file) else {
            return ReviewAction::Stay;
        };
        let line = file
            .lines
            .iter()
            .position(|line| line.starts_with("@@"))
            .and_then(|at| new_line(&file.lines, at));
        open(file, line)
    }

    pub fn open_row(&mut self, body: Rect, row: u16) -> ReviewAction {
        let line = self.diff_grid(body).grid.line_at(row);
        let Some(file) = self.files.get(self.file) else {
            return ReviewAction::Stay;
        };
        match usize::try_from(line)
            .ok()
            .and_then(|at| new_line(&file.lines, at))
        {
            Some(line) => open(file, Some(line)),
            None => ReviewAction::Stay,
        }
    }

    pub fn press(&mut self, body: Rect, col: u16, row: i32, now: Instant) {
        let DiffGrid { mut grid, selector } = self.diff_grid(body);
        selector.press(&mut grid, col, row, now);
    }

    pub fn extend(&mut self, body: Rect, col: u16, row: i32) {
        let DiffGrid { mut grid, selector } = self.diff_grid(body);
        selector.extend(&mut grid, col, row);
    }

    pub fn release(&mut self, body: Rect) -> Option<String> {
        let DiffGrid { mut grid, selector } = self.diff_grid(body);
        selector.release(&mut grid)
    }

    pub fn tick(&mut self, body: Rect) {
        let DiffGrid { mut grid, selector } = self.diff_grid(body);
        selector.tick(&mut grid);
    }

    fn diff_grid(&mut self, body: Rect) -> DiffGrid<'_> {
        let lines = self
            .files
            .get(self.file)
            .map_or(&[][..], |file| &file.lines);
        DiffGrid {
            grid: DiffBody {
                lines,
                scroll: &mut self.scroll,
                area: body,
            },
            selector: &mut self.selection,
        }
    }
}

fn open(file: &FileDiff, line: Option<u32>) -> ReviewAction {
    match file.deleted {
        true => ReviewAction::Notice(format!("{} was deleted", file.path)),
        false => ReviewAction::Open(EditorTarget {
            file: PathBuf::from(&file.path),
            line,
        }),
    }
}

struct DiffGrid<'a> {
    grid: DiffBody<'a>,
    selector: &'a mut Selector,
}

struct DiffBody<'a> {
    lines: &'a [String],
    scroll: &'a mut u16,
    area: Rect,
}

impl Grid for DiffBody<'_> {
    fn height(&self) -> u16 {
        self.area.height
    }

    fn width(&self) -> u16 {
        self.area.width
    }

    fn line_at(&self, row: u16) -> i64 {
        i64::from(*self.scroll) + i64::from(row)
    }

    fn scroll(&mut self, lines: isize) {
        let most = self
            .lines
            .len()
            .saturating_sub(usize::from(self.area.height));
        *self.scroll = scrolled_up(*self.scroll, lines, most);
    }

    fn row(&mut self, line: i64) -> Row {
        let cells = usize::try_from(line)
            .ok()
            .and_then(|at| self.lines.get(at))
            .map(|line| shown_cells(line, self.area.width))
            .unwrap_or_default();
        Row {
            cells,
            wrapped: false,
        }
    }
}

fn scrolled_up(from: u16, lines: isize, most: usize) -> u16 {
    let to = (from as isize - lines).clamp(0, most as isize);
    u16::try_from(to).unwrap_or(u16::MAX)
}

fn shown_cells(line: &str, width: u16) -> Vec<String> {
    let area = Rect::new(0, 0, width, 1);
    let mut buffer = Buffer::empty(area);
    Paragraph::new(line).render(area, &mut buffer);
    let mut cells = Vec::new();
    let mut hidden = 0;
    for x in 0..width {
        let symbol = buffer[(x, 0)].symbol();
        if hidden > 0 {
            hidden -= 1;
            cells.push(String::new());
            continue;
        }
        hidden = symbol.width().saturating_sub(1);
        cells.push(symbol.to_string());
    }
    cells
}

fn new_line(lines: &[String], at: usize) -> Option<u32> {
    let header = lines
        .get(..=at)?
        .iter()
        .rposition(|line| line.starts_with("@@"))?;
    let mut next = hunk_start(&lines[header])?;
    let mut line = next;
    for shown in &lines[header + 1..=at] {
        match shown.chars().next() {
            Some('-') => line = next,
            Some('\\') => {}
            _ => {
                line = next;
                next += 1;
            }
        }
    }
    Some(line.max(1))
}

fn hunk_start(header: &str) -> Option<u32> {
    let new = header.split(' ').find_map(|part| part.strip_prefix('+'))?;
    new.split(',').next()?.parse().ok()
}
