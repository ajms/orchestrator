use orch_term::keys::InputModes;
use serde::{Deserialize, Serialize};

use crate::Size;

const ALT_SCREEN_ENTRIES: [&[u8]; 2] = [b"\x1b[?1049h", b"\x1b[?47h"];
const ALT_SCREEN_ENTER: &[u8] = b"\x1b[?1049h";

pub struct Emulator {
    parser: vt100::Parser,
    main: Option<vt100::Screen>,
    carry: Vec<u8>,
}

impl Emulator {
    pub fn new(size: Size, scrollback: usize) -> Self {
        Self {
            parser: vt100::Parser::new(size.rows, size.cols, scrollback),
            main: None,
            carry: Vec::new(),
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        let mut data = std::mem::take(&mut self.carry);
        data.extend_from_slice(bytes);
        let mut rest = &data[..];
        while let Some((at, len)) = find_alt_entry(rest) {
            self.parser.process(&rest[..at]);
            if !self.parser.screen().alternate_screen() {
                self.main = Some(self.parser.screen().clone());
            }
            self.parser.process(&rest[at..at + len]);
            rest = &rest[at + len..];
        }
        let held = partial_alt_entry_len(rest);
        self.parser.process(&rest[..rest.len() - held]);
        self.carry = rest[rest.len() - held..].to_vec();
        if !self.parser.screen().alternate_screen() {
            self.main = None;
        }
    }

    pub fn resize(&mut self, size: Size) {
        self.parser.screen_mut().set_size(size.rows, size.cols);
        if let Some(main) = &mut self.main {
            main.set_size(size.rows, size.cols);
        }
    }

    pub fn input_modes(&self) -> InputModes {
        modes_of(self.parser.screen())
    }

    pub fn copy(&self) -> ScreenCopy {
        ScreenCopy {
            current: self.parser.screen().clone(),
            main: self.main.clone(),
        }
    }

    pub fn snapshot(&self) -> ScreenSnapshot {
        self.copy().capture()
    }
}

fn find_alt_entry(bytes: &[u8]) -> Option<(usize, usize)> {
    ALT_SCREEN_ENTRIES
        .iter()
        .filter_map(|entry| {
            bytes
                .windows(entry.len())
                .position(|window| window == *entry)
                .map(|at| (at, entry.len()))
        })
        .min()
}

fn partial_alt_entry_len(bytes: &[u8]) -> usize {
    ALT_SCREEN_ENTRIES
        .iter()
        .flat_map(|entry| (1..entry.len()).map(move |len| (entry, len)))
        .filter(|(entry, len)| bytes.ends_with(&entry[..*len]))
        .map(|(_, len)| len)
        .max()
        .unwrap_or(0)
}

fn modes_of(screen: &vt100::Screen) -> InputModes {
    InputModes {
        application_cursor: screen.application_cursor(),
        bracketed_paste: screen.bracketed_paste(),
    }
}

pub struct ScreenCopy {
    current: vt100::Screen,
    main: Option<vt100::Screen>,
}

impl ScreenCopy {
    pub fn capture(mut self) -> ScreenSnapshot {
        let (rows, cols) = self.current.size();
        let alternate = self
            .current
            .alternate_screen()
            .then(|| self.current.contents_formatted());
        let input_modes = self.current.input_mode_formatted();
        let main = match (&alternate, self.main.as_mut()) {
            (Some(_), Some(main)) => main,
            _ => &mut self.current,
        };
        ScreenSnapshot {
            size: Size { rows, cols },
            scrollback: scrollback_rows(main),
            screen: main.contents_formatted(),
            alternate,
            input_modes,
        }
    }
}

fn scrollback_rows(screen: &mut vt100::Screen) -> Vec<Vec<u8>> {
    let cols = screen.size().1;
    screen.set_scrollback(usize::MAX);
    let depth = screen.scrollback();
    let rows = (1..=depth)
        .rev()
        .map(|offset| {
            screen.set_scrollback(offset);
            screen.rows_formatted(0, cols).next().unwrap_or_default()
        })
        .collect();
    screen.set_scrollback(0);
    rows
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenSnapshot {
    pub size: Size,
    #[serde(with = "crate::bytes::list")]
    pub scrollback: Vec<Vec<u8>>,
    #[serde(with = "crate::bytes")]
    pub screen: Vec<u8>,
    #[serde(with = "crate::bytes::optional")]
    pub alternate: Option<Vec<u8>>,
    #[serde(with = "crate::bytes")]
    pub input_modes: Vec<u8>,
}

impl ScreenSnapshot {
    pub fn restore(&self, scrollback_len: usize) -> vt100::Parser {
        let Size { rows, cols } = self.size;
        let mut parser = vt100::Parser::new(rows, cols, scrollback_len);
        if !self.scrollback.is_empty() {
            parser.process(&self.scrollback.join(&b"\x1b[m\r\n"[..]));
            parser.process(&b"\n".repeat(usize::from(rows)));
            parser.process(b"\x1b[m");
        }
        parser.process(&self.screen);
        if let Some(alternate) = &self.alternate {
            parser.process(ALT_SCREEN_ENTER);
            parser.process(alternate);
        }
        parser.process(&self.input_modes);
        parser
    }

    pub fn alternate_screen(&self) -> bool {
        self.alternate.is_some()
    }

    pub fn input_modes(&self) -> InputModes {
        let mut parser = vt100::Parser::new(1, 1, 0);
        parser.process(&self.input_modes);
        modes_of(parser.screen())
    }

    pub fn text(&self) -> String {
        self.restore(0).screen().contents()
    }
}
