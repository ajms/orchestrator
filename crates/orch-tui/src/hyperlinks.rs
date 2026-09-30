use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::selection::{Grid, Point, columns_on, word_end, word_start};

const SCHEMES: [&str; 3] = ["https://", "http://", "file://"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HyperlinkTarget {
    Url(String),
    File { path: PathBuf, line: Option<u32> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hyperlink {
    pub start: Point,
    pub end: Point,
    pub target: HyperlinkTarget,
}

#[derive(Default)]
pub(crate) struct Hyperlinks {
    hovered: Option<Hyperlink>,
    pressed: bool,
}

impl Hyperlinks {
    pub fn hovered(&self) -> Option<&Hyperlink> {
        self.hovered.as_ref()
    }

    pub fn hover(&mut self, link: Option<Hyperlink>) {
        self.hovered = link;
    }

    pub fn shift(&mut self, lines: i64) {
        if let Some(link) = &mut self.hovered {
            link.start.0 -= lines;
            link.end.0 -= lines;
        }
    }

    pub fn press(&mut self) {
        self.pressed = true;
    }

    pub fn pressed(&self) -> bool {
        self.pressed
    }

    pub fn let_go(&mut self) {
        self.pressed = false;
    }
}

struct Token {
    text: String,
    cells: Vec<(Point, Range<usize>)>,
    pointer: usize,
}

impl Token {
    fn under(grid: &mut impl Grid, point: Point) -> Option<Self> {
        let start = word_start(grid, point);
        let end = word_end(grid, point);
        let mut token = Token {
            text: String::new(),
            cells: Vec::new(),
            pointer: 0,
        };
        for line in start.0..=end.0 {
            let row = grid.row(line);
            let (from, to) = columns_on(line, (start, end), grid.width() - 1);
            for col in from..=to {
                let cell = row.cells.get(usize::from(col))?;
                if (line, col) == point {
                    token.pointer = token.text.len();
                }
                let begin = token.text.len();
                token.text.push_str(cell);
                token.cells.push(((line, col), begin..token.text.len()));
            }
        }
        (!token.text.trim().is_empty()).then_some(token)
    }

    fn link(&self, span: Range<usize>, target: HyperlinkTarget) -> Option<Hyperlink> {
        if !span.contains(&self.pointer) {
            return None;
        }
        let covering = |byte: usize| {
            self.cells
                .iter()
                .find(|(_, range)| range.contains(&byte))
                .map(|(point, _)| *point)
        };
        Some(Hyperlink {
            start: covering(span.start)?,
            end: covering(span.end - 1)?,
            target,
        })
    }
}

pub(crate) fn hyperlink_at(
    grid: &mut impl Grid,
    point: Point,
    worktree: &Path,
) -> Option<Hyperlink> {
    let token = Token::under(grid, point)?;
    url(&token).or_else(|| file(&token, worktree))
}

fn url(token: &Token) -> Option<Hyperlink> {
    let start = SCHEMES
        .iter()
        .flat_map(|scheme| token.text.match_indices(scheme).map(|(at, _)| at))
        .filter(|at| *at <= token.pointer)
        .max()?;
    let url = trim_trailing(&token.text[start..]);
    if SCHEMES.contains(&url) {
        return None;
    }
    let span = start..start + url.len();
    token.link(span, HyperlinkTarget::Url(url.to_string()))
}

fn file(token: &Token, worktree: &Path) -> Option<Hyperlink> {
    let text = token.text.as_str();
    let start = text.len()
        - text
            .trim_start_matches(['(', '[', '{', '<', '"', '\'', '`'])
            .len();
    let location = trim_trailing(&text[start..]);
    let (path, line) = split_line(location);
    if path.is_empty() {
        return None;
    }
    let path = worktree.join(path);
    if !path.is_file() {
        return None;
    }
    let span = start..start + location.len();
    token.link(span, HyperlinkTarget::File { path, line })
}

fn split_line(location: &str) -> (&str, Option<u32>) {
    let mut path = location;
    let mut numbers = Vec::new();
    while numbers.len() < 2 {
        let Some((head, tail)) = path.rsplit_once(':') else {
            break;
        };
        let digits = !tail.is_empty() && tail.bytes().all(|byte| byte.is_ascii_digit());
        let Some(number) = tail.parse::<u32>().ok().filter(|_| digits) else {
            break;
        };
        numbers.push(number);
        path = head;
    }
    (path, numbers.last().copied())
}

fn trim_trailing(text: &str) -> &str {
    let mut text = text;
    while let Some(last) = text.chars().last() {
        let unmatched = |open: char, close: char| {
            last == close && text.matches(close).count() > text.matches(open).count()
        };
        let dangling = matches!(
            last,
            '.' | ',' | ';' | ':' | '!' | '?' | '"' | '\'' | '`' | '>'
        ) || unmatched('(', ')')
            || unmatched('[', ']')
            || unmatched('{', '}');
        if !dangling {
            break;
        }
        text = &text[..text.len() - last.len_utf8()];
    }
    text
}
