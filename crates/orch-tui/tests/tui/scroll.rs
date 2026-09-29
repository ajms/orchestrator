use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use crossterm::event::KeyCode;
use orch_protocol::FromDaemon;
use orch_tui::Effect;

use crate::common::*;

fn long_history() -> Harness {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first")]);
    let text: Vec<String> = (1..=100).map(|n| format!("line {n:03}")).collect();
    tui.pane(
        "first",
        FromDaemon::Output {
            bytes: text.join("\r\n").into_bytes(),
        },
    );
    tui.keys("l");
    tui
}

fn visible(tui: &mut Harness, needle: &str) -> bool {
    tui.screen().contains(needle)
}

fn clipboard(effects: &[Effect]) -> Option<String> {
    effects.iter().find_map(|effect| match effect {
        Effect::WriteTerminal(bytes) => {
            let text = String::from_utf8(bytes.clone()).ok()?;
            let payload = text.strip_prefix("\x1b]52;c;")?.strip_suffix('\x07')?;
            String::from_utf8(STANDARD.decode(payload).ok()?).ok()
        }
        _ => None,
    })
}

#[test]
fn j_and_k_scroll_the_pane_history_when_the_pane_has_focus() {
    let mut tui = long_history();
    assert!(visible(&mut tui, "line 100"));
    assert!(!visible(&mut tui, "line 073"));

    tui.keys("k");
    assert!(visible(&mut tui, "line 073"));
    assert!(!visible(&mut tui, "line 100"));
    tui.keys("j");
    assert!(visible(&mut tui, "line 100"));
}

#[test]
fn ctrl_u_and_ctrl_d_scroll_half_a_page() {
    let mut tui = long_history();
    tui.ctrl('u');
    assert!(visible(&mut tui, "line 061"));
    assert!(!visible(&mut tui, "line 100"));
    tui.ctrl('d');
    assert!(visible(&mut tui, "line 100"));
}

#[test]
fn gg_goes_to_the_top_of_the_history_and_capital_g_to_the_bottom() {
    let mut tui = long_history();
    tui.keys("gg");
    assert!(visible(&mut tui, "line 001"));
    assert!(!visible(&mut tui, "line 100"));
    tui.keys("G");
    assert!(visible(&mut tui, "line 100"));
}

#[test]
fn entering_insert_mode_returns_to_the_live_screen() {
    let mut tui = long_history();
    tui.keys("gg");
    tui.keys("i");
    assert!(visible(&mut tui, "line 100"));
}

#[test]
fn capital_v_selects_whole_lines_and_y_yanks_them_to_the_clipboard() {
    let mut tui = long_history();
    tui.keys("V");
    assert!(tui.lines().pop().unwrap().contains("V-LINE"));
    tui.keys("ky");

    assert_eq!(
        clipboard(&tui.take_effects()).as_deref(),
        Some("line 099\nline 100")
    );
    assert!(tui.lines().pop().unwrap().contains("NORMAL"));
}

#[test]
fn v_selects_characters_from_the_cursor() {
    let mut tui = long_history();
    tui.keys("v");
    assert!(tui.lines().pop().unwrap().contains("VISUAL"));
    tui.keys("hhhk");
    tui.keys("y");

    assert_eq!(
        clipboard(&tui.take_effects()).as_deref(),
        Some("099\nline 100")
    );
}

#[test]
fn esc_leaves_visual_mode_without_yanking() {
    let mut tui = long_history();
    tui.keys("V");
    tui.press(KeyCode::Esc);
    assert!(clipboard(&tui.take_effects()).is_none());
    assert!(tui.lines().pop().unwrap().contains("NORMAL"));
}

#[test]
fn the_visual_selection_is_highlighted_in_the_pane() {
    let mut tui = long_history();
    assert_eq!(tui.background_of("line 099"), ratatui::style::Color::Reset);
    tui.keys("Vk");
    assert_ne!(tui.background_of("line 099"), ratatui::style::Color::Reset);
    assert_ne!(tui.background_of("line 100"), ratatui::style::Color::Reset);
    assert_eq!(tui.background_of("line 098"), ratatui::style::Color::Reset);
}
