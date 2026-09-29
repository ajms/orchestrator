use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

use crate::mouse::PanePoint;

const LEGACY_COORD_OFFSET: u32 = 32;
const UTF8_MAX: u32 = 0x7ff;

pub(crate) fn encode(
    event: MouseEvent,
    at: PanePoint,
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
) -> Option<Vec<u8>> {
    if !wanted(event.kind, mode) {
        return None;
    }
    let modifiers = match mode {
        MouseProtocolMode::Press => 0,
        _ => modifier_bits(event.modifiers),
    };
    let code = button_code(event.kind, encoding) + modifiers;
    let (x, y) = (u32::from(at.col) + 1, u32::from(at.row) + 1);
    match encoding {
        MouseProtocolEncoding::Sgr => {
            let end = match event.kind {
                MouseEventKind::Up(_) => 'm',
                _ => 'M',
            };
            Some(format!("\x1b[<{code};{x};{y}{end}").into_bytes())
        }
        MouseProtocolEncoding::Utf8 => legacy([code, x, y], utf8),
        MouseProtocolEncoding::Default => legacy([code, x, y], byte),
    }
}

fn wanted(kind: MouseEventKind, mode: MouseProtocolMode) -> bool {
    use MouseProtocolMode::{AnyMotion, ButtonMotion, None, PressRelease};
    match kind {
        MouseEventKind::Up(_) => matches!(mode, PressRelease | ButtonMotion | AnyMotion),
        MouseEventKind::Drag(_) => matches!(mode, ButtonMotion | AnyMotion),
        MouseEventKind::Moved => mode == AnyMotion,
        _ => mode != None,
    }
}

fn button_code(kind: MouseEventKind, encoding: MouseProtocolEncoding) -> u32 {
    match kind {
        MouseEventKind::Down(button) => button_number(button),
        MouseEventKind::Up(button) if encoding == MouseProtocolEncoding::Sgr => {
            button_number(button)
        }
        MouseEventKind::Up(_) => 3,
        MouseEventKind::Drag(button) => button_number(button) + 32,
        MouseEventKind::Moved => 35,
        MouseEventKind::ScrollUp => 64,
        MouseEventKind::ScrollDown => 65,
        MouseEventKind::ScrollLeft => 66,
        MouseEventKind::ScrollRight => 67,
    }
}

fn button_number(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

fn modifier_bits(modifiers: KeyModifiers) -> u32 {
    [
        (KeyModifiers::SHIFT, 4),
        (KeyModifiers::ALT, 8),
        (KeyModifiers::CONTROL, 16),
    ]
    .into_iter()
    .filter(|(modifier, _)| modifiers.contains(*modifier))
    .map(|(_, bit)| bit)
    .sum()
}

fn legacy(values: [u32; 3], write: fn(&mut Vec<u8>, u32) -> Option<()>) -> Option<Vec<u8>> {
    let mut out = b"\x1b[M".to_vec();
    for value in values {
        write(&mut out, value + LEGACY_COORD_OFFSET)?;
    }
    Some(out)
}

fn byte(out: &mut Vec<u8>, value: u32) -> Option<()> {
    out.push(u8::try_from(value).ok()?);
    Some(())
}

fn utf8(out: &mut Vec<u8>, value: u32) -> Option<()> {
    let c = char::from_u32(value).filter(|_| value <= UTF8_MAX)?;
    out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
    Some(())
}
