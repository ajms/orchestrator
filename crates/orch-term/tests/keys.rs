use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use orch_term::keys::{InputModes, encode_key, encode_paste, is_ctrl_backslash};

const NORMAL: InputModes = InputModes {
    application_cursor: false,
    bracketed_paste: false,
};
const APP: InputModes = InputModes {
    application_cursor: true,
    bracketed_paste: true,
};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

#[test]
fn printable_characters_are_sent_as_utf8() {
    assert_eq!(encode_key(key(KeyCode::Char('a')), NORMAL), b"a");
    assert_eq!(encode_key(key(KeyCode::Char('é')), NORMAL), "é".as_bytes());
    assert_eq!(
        encode_key(
            KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT),
            NORMAL
        ),
        b"A"
    );
}

#[test]
fn control_letters_map_to_c0_codes() {
    assert_eq!(encode_key(ctrl('c'), NORMAL), [0x03]);
    assert_eq!(encode_key(ctrl('A'), NORMAL), [0x01]);
    assert_eq!(encode_key(ctrl(' '), NORMAL), [0x00]);
    assert_eq!(encode_key(ctrl('['), NORMAL), [0x1b]);
}

#[test]
fn ctrl_backslash_and_ctrl_4_both_send_0x1c() {
    assert_eq!(encode_key(ctrl('\\'), NORMAL), [0x1c]);
    assert_eq!(encode_key(ctrl('4'), NORMAL), [0x1c]);
    assert!(is_ctrl_backslash(ctrl('\\')));
    assert!(is_ctrl_backslash(ctrl('4')));
    assert!(!is_ctrl_backslash(key(KeyCode::Char('\\'))));
    assert!(!is_ctrl_backslash(ctrl('n')));
}

#[test]
fn arrows_honour_application_cursor_mode() {
    assert_eq!(encode_key(key(KeyCode::Up), NORMAL), b"\x1b[A");
    assert_eq!(encode_key(key(KeyCode::Left), NORMAL), b"\x1b[D");
    assert_eq!(encode_key(key(KeyCode::Up), APP), b"\x1bOA");
    assert_eq!(encode_key(key(KeyCode::Right), APP), b"\x1bOC");
}

#[test]
fn editing_and_function_keys() {
    assert_eq!(encode_key(key(KeyCode::Enter), NORMAL), b"\r");
    assert_eq!(encode_key(key(KeyCode::Tab), NORMAL), b"\t");
    assert_eq!(encode_key(key(KeyCode::BackTab), NORMAL), b"\x1b[Z");
    assert_eq!(encode_key(key(KeyCode::Backspace), NORMAL), [0x7f]);
    assert_eq!(encode_key(key(KeyCode::Esc), NORMAL), [0x1b]);
    assert_eq!(encode_key(key(KeyCode::Delete), NORMAL), b"\x1b[3~");
    assert_eq!(encode_key(key(KeyCode::PageUp), NORMAL), b"\x1b[5~");
    assert_eq!(encode_key(key(KeyCode::F(1)), NORMAL), b"\x1bOP");
    assert_eq!(encode_key(key(KeyCode::F(5)), NORMAL), b"\x1b[15~");
    assert_eq!(encode_key(key(KeyCode::Home), APP), b"\x1bOH");
    assert_eq!(encode_key(key(KeyCode::End), NORMAL), b"\x1b[F");
}

#[test]
fn alt_prefixes_escape_and_shift_enter_inserts_newline() {
    assert_eq!(
        encode_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT), NORMAL),
        b"\x1bb"
    );
    assert_eq!(
        encode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT), NORMAL),
        b"\x1b\r"
    );
}

#[test]
fn keys_without_encoding_send_nothing() {
    assert!(encode_key(key(KeyCode::CapsLock), NORMAL).is_empty());
}

#[test]
fn paste_is_bracketed_only_when_the_agent_enabled_it() {
    assert_eq!(encode_paste("hi\nthere", NORMAL), b"hi\nthere");
    assert_eq!(encode_paste("hi", APP), b"\x1b[200~hi\x1b[201~");
}

#[test]
fn bracketed_paste_cannot_be_terminated_early_by_its_content() {
    assert_eq!(encode_paste("a\x1b[201~b", APP), b"\x1b[200~ab\x1b[201~");
}
