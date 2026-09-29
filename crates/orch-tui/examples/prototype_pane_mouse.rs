// PROTOTYPE — throwaway, answers "does the Session pane feel exactly like plain Claude Code?" (issue #25).
// Run: cargo run -p orch-tui --example prototype_pane_mouse -- claude
//      cargo run -p orch-tui --example prototype_pane_mouse -- env CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN=1 claude
// Quit with Ctrl-\.

use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use base64::Engine;
use crossterm::event::{
    self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, Event, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use orch_term::keys::{InputModes, encode_key, encode_paste, is_ctrl_backslash};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Color;
use ratatui::widgets::{Block, Paragraph};
use tui_term::widget::PseudoTerminal;
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

const SIDEBAR: u16 = 34;
const SCROLLBACK: usize = 10_000;
const MULTI_CLICK: Duration = Duration::from_millis(400);
const SELECTION: Color = Color::Rgb(70, 70, 110);

#[derive(Default)]
struct Clip {
    pending: Vec<(Vec<u8>, Vec<u8>)>,
}

impl vt100::Callbacks for Clip {
    fn copy_to_clipboard(&mut self, _: &mut vt100::Screen, ty: &[u8], data: &[u8]) {
        self.pending.push((ty.to_vec(), data.to_vec()));
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Unit {
    Char,
    Word,
    Line,
}

#[derive(Clone, Copy)]
struct Sel {
    anchor: (i64, u16),
    cursor: (i64, u16),
    unit: Unit,
}

struct State {
    parser: vt100::Parser<Clip>,
    writer: Box<dyn Write + Send>,
    inner: Rect,
    captured: bool,
    sel: Option<Sel>,
    last_click: Option<(Instant, u16, u16, u8)>,
    focused: bool,
    log: Vec<String>,
    toast: String,
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let argv = if args.is_empty() { vec!["claude".to_string()] } else { args };

    let (cols, rows) = crossterm::terminal::size()?;
    let inner = pane_inner(Rect::new(0, 0, cols, rows));
    let pty = native_pty_system()
        .openpty(PtySize { rows: inner.height, cols: inner.width, pixel_width: 0, pixel_height: 0 })
        .map_err(io::Error::other)?;
    let mut command = CommandBuilder::new(&argv[0]);
    command.args(&argv[1..]);
    command.cwd(std::env::current_dir()?);
    command.env("TERM", "xterm-256color");
    let mut child = pty.slave.spawn_command(command).map_err(io::Error::other)?;
    drop(pty.slave);
    let mut reader = pty.master.try_clone_reader().map_err(io::Error::other)?;
    let writer = pty.master.take_writer().map_err(io::Error::other)?;
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture, EnableFocusChange, EnableBracketedPaste)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut st = State {
        parser: vt100::Parser::new_with_callbacks(inner.height, inner.width, SCROLLBACK, Clip::default()),
        writer,
        inner,
        captured: false,
        sel: None,
        last_click: None,
        focused: true,
        log: Vec::new(),
        toast: String::new(),
    };

    let result = (|| -> io::Result<()> {
        loop {
            while let Ok(bytes) = rx.try_recv() {
                st.parser.process(&bytes);
            }
            for (ty, data) in std::mem::take(&mut st.parser.callbacks_mut().pending) {
                let mut out = io::stdout();
                write!(out, "\x1b]52;{};{}\x07", String::from_utf8_lossy(&ty), String::from_utf8_lossy(&data))?;
                out.flush()?;
                st.toast = format!("Agent OSC 52 relayed ({} b64 bytes)", data.len());
            }
            if let Ok(Some(_)) = child.try_wait() {
                return Ok(());
            }
            terminal.draw(|frame| draw(frame, &mut st))?;
            if !event::poll(Duration::from_millis(16))? {
                continue;
            }
            match event::read()? {
                Event::Key(key) if is_ctrl_backslash(key) => return Ok(()),
                Event::Key(key) => {
                    st.parser.screen_mut().set_scrollback(0);
                    let bytes = encode_key(key, modes(&st));
                    st.send(&bytes);
                }
                Event::Paste(text) => {
                    let bytes = encode_paste(&text, modes(&st));
                    st.send(&bytes);
                }
                Event::FocusGained => {
                    st.focused = true;
                    st.send(b"\x1b[I");
                }
                Event::FocusLost => {
                    st.focused = false;
                    st.send(b"\x1b[O");
                }
                Event::Mouse(mouse) => st.mouse(mouse),
                Event::Resize(cols, rows) => {
                    st.inner = pane_inner(Rect::new(0, 0, cols, rows));
                    st.parser.screen_mut().set_size(st.inner.height, st.inner.width);
                    let _ = pty.master.resize(PtySize {
                        rows: st.inner.height,
                        cols: st.inner.width,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                }
                _ => {}
            }
        }
    })();

    execute!(io::stdout(), DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, LeaveAlternateScreen)?;
    disable_raw_mode()?;
    let _ = child.kill();
    result
}

fn modes(st: &State) -> InputModes {
    let screen = st.parser.screen();
    InputModes {
        application_cursor: screen.application_cursor(),
        bracketed_paste: screen.bracketed_paste(),
    }
}

fn pane_inner(area: Rect) -> Rect {
    let [_, pane] = Layout::horizontal([Constraint::Length(SIDEBAR), Constraint::Min(10)]).areas(area);
    Block::bordered().inner(pane)
}

impl State {
    fn send(&mut self, bytes: &[u8]) {
        let _ = self.writer.write_all(bytes);
        let _ = self.writer.flush();
    }

    fn note(&mut self, line: String) {
        self.log.push(line);
        if self.log.len() > 8 {
            self.log.remove(0);
        }
    }

    fn contains(&self, col: u16, row: u16) -> bool {
        let r = self.inner;
        col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
    }

    fn clamp(&self, col: u16, row: u16) -> (u16, u16) {
        let r = self.inner;
        (
            col.clamp(r.x, r.x + r.width - 1) - r.x,
            row.clamp(r.y, r.y + r.height - 1) - r.y,
        )
    }

    fn mouse(&mut self, mouse: MouseEvent) {
        let starts = matches!(mouse.kind, MouseEventKind::Down(_));
        let wheel = matches!(mouse.kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown);
        let inside = self.contains(mouse.column, mouse.row);
        if starts && inside {
            self.captured = true;
        }
        let ours = self.captured || ((wheel || matches!(mouse.kind, MouseEventKind::Moved)) && inside);
        if !ours {
            return;
        }
        if matches!(mouse.kind, MouseEventKind::Up(_)) {
            self.captured = false;
        }
        let mode = self.parser.screen().mouse_protocol_mode();
        if mode == MouseProtocolMode::None {
            self.select(mouse);
        } else {
            self.forward(mouse, mode);
        }
    }

    fn forward(&mut self, mouse: MouseEvent, mode: MouseProtocolMode) {
        let wanted = match mouse.kind {
            MouseEventKind::Down(_) | MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => true,
            MouseEventKind::Up(_) => mode != MouseProtocolMode::Press,
            MouseEventKind::Drag(_) => matches!(mode, MouseProtocolMode::ButtonMotion | MouseProtocolMode::AnyMotion),
            MouseEventKind::Moved => mode == MouseProtocolMode::AnyMotion,
            _ => false,
        };
        if !wanted {
            return;
        }
        let (x, y) = self.clamp(mouse.column, mouse.row);
        let bytes = encode_mouse(mouse, x, y, self.parser.screen().mouse_protocol_encoding());
        if !matches!(mouse.kind, MouseEventKind::Moved) {
            self.note(format!("→ {:?} {}", mouse.kind, String::from_utf8_lossy(&bytes[1..])));
        }
        self.send(&bytes);
    }

    fn select(&mut self, mouse: MouseEvent) {
        let (x, y) = self.clamp(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::ScrollUp => self.scroll(3),
            MouseEventKind::ScrollDown => self.scroll(-3),
            MouseEventKind::Down(MouseButton::Left) => {
                let clicks = match self.last_click {
                    Some((at, cx, cy, n)) if at.elapsed() < MULTI_CLICK && (cx, cy) == (x, y) => n % 3 + 1,
                    _ => 1,
                };
                self.last_click = Some((Instant::now(), x, y, clicks));
                let line = self.abs(y);
                let unit = [Unit::Char, Unit::Word, Unit::Line][usize::from(clicks - 1)];
                self.sel = Some(Sel { anchor: (line, x), cursor: (line, x), unit });
                if unit == Unit::Char {
                    self.toast.clear();
                }
                self.note(format!("click×{clicks} at {x},{y}"));
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if mouse.row < self.inner.y {
                    self.scroll(1);
                } else if mouse.row >= self.inner.y + self.inner.height {
                    self.scroll(-1);
                }
                let line = self.abs(y);
                if let Some(sel) = &mut self.sel {
                    sel.cursor = (line, x);
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let text = self.sel.map(|sel| self.text(sel)).unwrap_or_default();
                match self.sel {
                    Some(sel) if sel.unit == Unit::Char && sel.anchor == sel.cursor => self.sel = None,
                    _ if !text.is_empty() => self.copy(&text),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn scroll(&mut self, by: isize) {
        let screen = self.parser.screen_mut();
        let next = screen.scrollback().saturating_add_signed(by);
        screen.set_scrollback(next);
    }

    fn abs(&self, row: u16) -> i64 {
        i64::from(row) - self.parser.screen().scrollback() as i64
    }

    fn row_chars(&mut self, line: i64) -> (Vec<String>, bool) {
        let screen = self.parser.screen_mut();
        let kept = screen.scrollback();
        screen.set_scrollback((-line).max(0) as usize);
        let row = (line + screen.scrollback() as i64).max(0) as u16;
        let (_, cols) = screen.size();
        let cells = (0..cols)
            .map(|col| screen.cell(row, col).map(|c| c.contents().to_string()).unwrap_or_default())
            .collect();
        let wrapped = screen.row_wrapped(row);
        screen.set_scrollback(kept);
        (cells, wrapped)
    }

    fn span(&mut self, sel: Sel) -> ((i64, u16), (i64, u16)) {
        let (mut start, mut end) = if sel.anchor <= sel.cursor { (sel.anchor, sel.cursor) } else { (sel.cursor, sel.anchor) };
        match sel.unit {
            Unit::Char => {}
            Unit::Line => {
                start.1 = 0;
                end.1 = self.inner.width - 1;
            }
            Unit::Word => {
                let blank = |c: &String| c.is_empty() || c.trim().is_empty();
                let (cells, _) = self.row_chars(start.0);
                while start.1 > 0 && !blank(&cells[usize::from(start.1 - 1)]) && !blank(&cells[usize::from(start.1)]) {
                    start.1 -= 1;
                }
                let (cells, _) = self.row_chars(end.0);
                while usize::from(end.1) + 1 < cells.len() && !blank(&cells[usize::from(end.1 + 1)]) && !blank(&cells[usize::from(end.1)]) {
                    end.1 += 1;
                }
            }
        }
        (start, end)
    }

    fn text(&mut self, sel: Sel) -> String {
        let (start, end) = self.span(sel);
        let mut out = String::new();
        for line in start.0..=end.0 {
            let (cells, wrapped) = self.row_chars(line);
            let from = if line == start.0 { usize::from(start.1) } else { 0 };
            let to = if line == end.0 { usize::from(end.1) + 1 } else { cells.len() };
            let piece: String = cells[from.min(cells.len())..to.min(cells.len())]
                .iter()
                .map(|c| if c.is_empty() { " " } else { c.as_str() })
                .collect();
            out.push_str(piece.trim_end());
            if line != end.0 && !wrapped {
                out.push('\n');
            }
        }
        out
    }

    fn copy(&mut self, text: &str) {
        let native = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            Some(("wl-copy", vec![]))
        } else if std::env::var_os("DISPLAY").is_some() {
            Some(("xclip", vec!["-selection", "clipboard"]))
        } else {
            None
        };
        let via = native
            .and_then(|(bin, args)| {
                let mut child = Command::new(bin).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().ok()?;
                child.stdin.take()?.write_all(text.as_bytes()).ok()?;
                Some(bin)
            })
            .unwrap_or_else(|| {
                let b64 = base64::engine::general_purpose::STANDARD.encode(text);
                let mut out = io::stdout();
                let _ = write!(out, "\x1b]52;c;{b64}\x07");
                let _ = out.flush();
                "OSC 52"
            });
        self.toast = format!("copied {} chars via {via}", text.chars().count());
    }
}

fn encode_mouse(mouse: MouseEvent, x: u16, y: u16, encoding: MouseProtocolEncoding) -> Vec<u8> {
    let button = |b: MouseButton| match b {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    };
    let (mut code, release) = match mouse.kind {
        MouseEventKind::Down(b) => (button(b), false),
        MouseEventKind::Up(b) => (if encoding == MouseProtocolEncoding::Sgr { button(b) } else { 3 }, true),
        MouseEventKind::Drag(b) => (button(b) + 32, false),
        MouseEventKind::Moved => (35, false),
        MouseEventKind::ScrollUp => (64, false),
        MouseEventKind::ScrollDown => (65, false),
        MouseEventKind::ScrollLeft => (66, false),
        MouseEventKind::ScrollRight => (67, false),
    };
    if mouse.modifiers.contains(KeyModifiers::SHIFT) {
        code += 4;
    }
    if mouse.modifiers.contains(KeyModifiers::ALT) {
        code += 8;
    }
    if mouse.modifiers.contains(KeyModifiers::CONTROL) {
        code += 16;
    }
    let (cx, cy) = (u32::from(x) + 1, u32::from(y) + 1);
    match encoding {
        MouseProtocolEncoding::Sgr => {
            format!("\x1b[<{code};{cx};{cy}{}", if release { 'm' } else { 'M' }).into_bytes()
        }
        MouseProtocolEncoding::Utf8 => {
            let mut out = b"\x1b[M".to_vec();
            for v in [code + 32, cx + 32, cy + 32] {
                let mut buf = [0u8; 4];
                out.extend_from_slice(char::from_u32(v).unwrap_or(' ').encode_utf8(&mut buf).as_bytes());
            }
            out
        }
        MouseProtocolEncoding::Default => {
            let clip = |v: u32| (v + 32).min(255) as u8;
            vec![0x1b, b'[', b'M', clip(code), clip(cx), clip(cy)]
        }
    }
}

fn draw(frame: &mut ratatui::Frame, st: &mut State) {
    let [side, pane] = Layout::horizontal([Constraint::Length(SIDEBAR), Constraint::Min(10)]).areas(frame.area());
    let screen = st.parser.screen();
    let mode = screen.mouse_protocol_mode();
    let owner = if mode == MouseProtocolMode::None { "Orchestrator (Pane selection)" } else { "Agent (passthrough)" };
    let mut lines = vec![
        "PROTOTYPE #25 — Ctrl-\\ quits".to_string(),
        String::new(),
        format!("mouse owner: {owner}"),
        format!("agent mode: {mode:?}"),
        format!("encoding:   {:?}", screen.mouse_protocol_encoding()),
        format!("alt screen: {}", screen.alternate_screen()),
        format!("scrollback: {}", screen.scrollback()),
        format!("focused:    {}", st.focused),
        format!("gesture:    {}", if st.captured { "held by pane" } else { "-" }),
        format!("selection:  {}", st.sel.map_or("-".into(), |s| format!("{:?}→{:?}", s.anchor, s.cursor))),
        String::new(),
        format!("toast: {}", st.toast),
        String::new(),
        "last events:".to_string(),
    ];
    lines.extend(st.log.iter().cloned());
    frame.render_widget(Paragraph::new(lines.join("\n")).block(Block::bordered().title(" state ")), side);
    frame.render_widget(PseudoTerminal::new(st.parser.screen()).block(Block::bordered().title(" Session pane ")), pane);

    if let Some(sel) = st.sel {
        let (start, end) = st.span(sel);
        let offset = st.parser.screen().scrollback() as i64;
        let inner = st.inner;
        let buffer = frame.buffer_mut();
        for line in start.0..=end.0 {
            let row = line + offset;
            if row < 0 || row >= i64::from(inner.height) {
                continue;
            }
            let from = if line == start.0 { start.1 } else { 0 };
            let to = if line == end.0 { end.1 } else { inner.width - 1 };
            for col in from..=to.min(inner.width - 1) {
                buffer[(inner.x + col, inner.y + row as u16)].set_bg(SELECTION);
            }
        }
    }
}
