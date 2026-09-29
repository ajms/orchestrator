use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const ESC: u8 = 0x1b;
const PASTE_START: &str = "\x1b[200~";
const PASTE_END: &str = "\x1b[201~";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputModes {
    pub application_cursor: bool,
    pub bracketed_paste: bool,
}

pub fn is_ctrl_backslash(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('\\' | '4'))
}

pub fn encode_key(key: KeyEvent, modes: InputModes) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let cursor = |c: u8| {
        let intro = if modes.application_cursor { b'O' } else { b'[' };
        vec![ESC, intro, c]
    };
    let tilde = |n: u8| format!("\x1b[{n}~").into_bytes();
    let mut bytes = match key.code {
        KeyCode::Char(c) if ctrl => control_char(c),
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter if shift || alt => return vec![ESC, b'\r'],
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![ESC],
        KeyCode::Up => cursor(b'A'),
        KeyCode::Down => cursor(b'B'),
        KeyCode::Right => cursor(b'C'),
        KeyCode::Left => cursor(b'D'),
        KeyCode::Home => cursor(b'H'),
        KeyCode::End => cursor(b'F'),
        KeyCode::Insert => tilde(2),
        KeyCode::Delete => tilde(3),
        KeyCode::PageUp => tilde(5),
        KeyCode::PageDown => tilde(6),
        KeyCode::F(n @ 1..=4) => vec![ESC, b'O', b'P' + n - 1],
        KeyCode::F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)]),
        _ => Vec::new(),
    };
    if alt && !bytes.is_empty() {
        bytes.insert(0, ESC);
    }
    bytes
}

fn control_char(c: char) -> Vec<u8> {
    let code = match c.to_ascii_lowercase() {
        c @ 'a'..='z' => c as u8 - b'a' + 1,
        ' ' | '2' | '@' => 0x00,
        '[' | '3' => ESC,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '7' | '/' => 0x1f,
        '8' | '?' => 0x7f,
        other => return other.to_string().into_bytes(),
    };
    vec![code]
}

pub fn encode_paste(text: &str, modes: InputModes) -> Vec<u8> {
    if !modes.bracketed_paste {
        return text.as_bytes().to_vec();
    }
    let body = text.replace(PASTE_END, "");
    format!("{PASTE_START}{body}{PASTE_END}").into_bytes()
}
