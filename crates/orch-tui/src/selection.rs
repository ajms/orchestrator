use std::time::{Duration, Instant};

const MULTI_CLICK: Duration = Duration::from_millis(400);

pub(crate) type Point = (i64, u16);

#[derive(PartialEq, Eq)]
pub(crate) struct Row {
    pub cells: Vec<String>,
    pub wrapped: bool,
}

impl Row {
    fn blank_at(&self, col: u16) -> bool {
        self.cells
            .get(usize::from(col))
            .is_none_or(|cell| cell == " ")
    }

    fn text(&self, from: u16, to: u16) -> String {
        let to = usize::from(to).min(self.cells.len());
        let from = usize::from(from).min(to);
        self.cells[from..to].concat()
    }
}

pub(crate) trait Grid {
    fn height(&self) -> u16;
    fn width(&self) -> u16;
    fn line_at(&self, row: u16) -> i64;
    fn scroll(&mut self, lines: isize);
    fn row(&mut self, line: i64) -> Row;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Char,
    Word,
    Line,
}

#[derive(Debug, Clone, Copy)]
struct Press {
    at: Instant,
    point: Point,
    count: u8,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    anchor: Point,
    cursor: Point,
    unit: Unit,
}

#[derive(Default)]
pub(crate) struct Selector {
    drag: Option<Drag>,
    span: Option<(Point, Point)>,
    held: bool,
    past_edge: Option<(u16, i32)>,
    pinned: bool,
    last_press: Option<Press>,
}

impl Selector {
    pub fn span(&self) -> Option<(Point, Point)> {
        self.span
    }

    pub fn clear(&mut self) {
        self.drag = None;
        self.span = None;
        self.let_go();
    }

    pub fn shift(&mut self, lines: i64) {
        let moved = |(line, col): Point| (line - lines, col);
        if let Some(drag) = &mut self.drag {
            drag.anchor = moved(drag.anchor);
            drag.cursor = moved(drag.cursor);
        }
        self.span = self.span.map(|(start, end)| (moved(start), moved(end)));
        if let Some(press) = &mut self.last_press {
            press.point = moved(press.point);
        }
    }

    pub fn press(&mut self, grid: &mut impl Grid, col: u16, row: i32, now: Instant) {
        let point = (grid.line_at(clamp_row(grid, row)), col);
        let count = match self.last_press {
            Some(last) if last.point == point && now.duration_since(last.at) < MULTI_CLICK => {
                last.count % 3 + 1
            }
            _ => 1,
        };
        self.last_press = Some(Press {
            at: now,
            point,
            count,
        });
        let unit = [Unit::Char, Unit::Word, Unit::Line][usize::from(count - 1)];
        self.drag = Some(Drag {
            anchor: point,
            cursor: point,
            unit,
        });
        self.held = true;
        self.past_edge = None;
        self.span = Some(expand(grid, point, point, unit));
    }

    pub fn extend(&mut self, grid: &mut impl Grid, col: u16, row: i32) {
        if !self.held {
            return;
        }
        let beyond = row < 0 || row >= i32::from(grid.height());
        self.past_edge = beyond.then_some((col, row));
        self.follow(grid, col, row);
    }

    pub fn auto_scrolling(&self) -> bool {
        self.held && self.past_edge.is_some() && !self.pinned
    }

    pub fn let_go(&mut self) {
        self.held = false;
        self.past_edge = None;
    }

    pub fn tick(&mut self, grid: &mut impl Grid) {
        if let Some((col, row)) = self.past_edge.filter(|_| self.held) {
            self.follow(grid, col, row);
        }
    }

    pub fn release(&mut self, grid: &mut impl Grid) -> Option<String> {
        if !self.held {
            return None;
        }
        self.let_go();
        let drag = self.drag?;
        if drag.unit == Unit::Char && drag.anchor == drag.cursor {
            self.clear();
            return None;
        }
        let (start, end) = self.span?;
        let text = text_between(grid, start, end);
        (!text.trim().is_empty()).then_some(text)
    }

    fn follow(&mut self, grid: &mut impl Grid, col: u16, row: i32) {
        let Some(drag) = &mut self.drag else {
            return;
        };
        let top = grid.line_at(0);
        if row < 0 {
            grid.scroll(1);
        } else if row >= i32::from(grid.height()) {
            grid.scroll(-1);
        }
        self.pinned = self.past_edge.is_some() && grid.line_at(0) == top;
        drag.cursor = (grid.line_at(clamp_row(grid, row)), col);
        let (anchor, cursor, unit) = (drag.anchor, drag.cursor, drag.unit);
        self.span = Some(expand(grid, anchor, cursor, unit));
    }
}

fn clamp_row(grid: &impl Grid, row: i32) -> u16 {
    let last = i32::from(grid.height().saturating_sub(1));
    row.clamp(0, last) as u16
}

fn expand(grid: &mut impl Grid, anchor: Point, cursor: Point, unit: Unit) -> (Point, Point) {
    let (start, end) = match anchor <= cursor {
        true => (anchor, cursor),
        false => (cursor, anchor),
    };
    match unit {
        Unit::Char => (start, end),
        Unit::Word => (word_start(grid, start), word_end(grid, end)),
        Unit::Line => (
            (logical_start(grid, start.0), 0),
            (logical_end(grid, end.0), grid.width().saturating_sub(1)),
        ),
    }
}

fn word_start(grid: &mut impl Grid, (mut line, mut col): Point) -> Point {
    let mut row = grid.row(line);
    if row.blank_at(col) {
        return (line, col);
    }
    loop {
        if col > 0 && !row.blank_at(col - 1) {
            col -= 1;
            continue;
        }
        if col > 0 {
            return (line, col);
        }
        let above = grid.row(line - 1);
        let last = grid.width().saturating_sub(1);
        if !above.wrapped || above.blank_at(last) {
            return (line, col);
        }
        (line, col, row) = (line - 1, last, above);
    }
}

fn word_end(grid: &mut impl Grid, (mut line, mut col): Point) -> Point {
    let mut row = grid.row(line);
    if row.blank_at(col) {
        return (line, col);
    }
    let last = grid.width().saturating_sub(1);
    loop {
        if col < last && !row.blank_at(col + 1) {
            col += 1;
            continue;
        }
        if col < last || !row.wrapped {
            return (line, col);
        }
        let below = grid.row(line + 1);
        if below.blank_at(0) {
            return (line, col);
        }
        (line, col, row) = (line + 1, 0, below);
    }
}

fn logical_start(grid: &mut impl Grid, mut line: i64) -> i64 {
    while grid.row(line - 1).wrapped {
        line -= 1;
    }
    line
}

fn logical_end(grid: &mut impl Grid, mut line: i64) -> i64 {
    while grid.row(line).wrapped {
        line += 1;
    }
    line
}

pub(crate) fn text_between(grid: &mut impl Grid, start: Point, end: Point) -> String {
    let mut text = String::new();
    for line in start.0..=end.0 {
        let row = grid.row(line);
        let from = if line == start.0 { start.1 } else { 0 };
        let to = if line == end.0 {
            end.1.saturating_add(1)
        } else {
            u16::MAX
        };
        let piece = row.text(from, to);
        match row.wrapped && line != end.0 {
            true => text.push_str(&piece),
            false => text.push_str(piece.trim_end()),
        }
        if line != end.0 && !row.wrapped {
            text.push('\n');
        }
    }
    text
}
