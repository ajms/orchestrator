use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use crossterm::event::MouseEventKind;
use orch_protocol::DisplayVars;
use orch_protocol::FromDaemon;
use orch_tui::{Effect, TuiConfig};
use ratatui::style::Color;

use crate::common::*;

const BELOW_PANE: u16 = PANE_TOP + 27;

fn pane_with(text: &str) -> Harness {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first")]);
    output(&mut tui, text);
    tui.take_effects();
    tui
}

fn output(tui: &mut Harness, text: &str) {
    let bytes = text.as_bytes().to_vec();
    tui.pane("first", FromDaemon::Output { bytes });
}

fn long_history() -> Harness {
    let text: Vec<String> = (1..=100).map(|n| format!("line {n:03}")).collect();
    pane_with(&text.join("\r\n"))
}

fn lines(from: u32, to: u32) -> String {
    (from..=to)
        .map(|n| format!("line {n:03}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn copies(tui: &mut Harness) -> Vec<String> {
    tui.take_effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::WriteTerminal(bytes) => {
                let text = String::from_utf8(bytes.clone()).ok()?;
                let payload = text.strip_prefix("\x1b]52;c;")?.strip_suffix('\x07')?;
                String::from_utf8(STANDARD.decode(payload).ok()?).ok()
            }
            _ => None,
        })
        .collect()
}

fn last_copy(tui: &mut Harness) -> Option<String> {
    copies(tui).pop()
}

fn down(tui: &mut Harness, col: u16, row: u16) {
    let (x, y) = in_pane(col, row);
    tui.mouse_down(x, y);
}

fn drag(tui: &mut Harness, col: u16, row: u16) {
    let (x, y) = in_pane(col, row);
    tui.drag(x, y);
}

fn up(tui: &mut Harness, col: u16, row: u16) {
    let (x, y) = in_pane(col, row);
    tui.release(x, y);
}

fn click(tui: &mut Harness, col: u16, row: u16) {
    down(tui, col, row);
    up(tui, col, row);
}

fn select(tui: &mut Harness, from: (u16, u16), to: (u16, u16)) {
    down(tui, from.0, from.1);
    drag(tui, to.0, to.1);
    up(tui, to.0, to.1);
}

fn wheel(tui: &mut Harness, kind: MouseEventKind, times: usize) {
    let (x, y) = in_pane(5, 5);
    for _ in 0..times {
        tui.wheel(kind, x, y);
    }
}

fn visible(tui: &mut Harness, needle: &str) -> bool {
    tui.screen().contains(needle)
}

fn visual_colour() -> Color {
    let mut tui = pane_with("hello");
    tui.keys("lV");
    tui.background_of("hello")
}

#[test]
fn a_drag_selects_characters_and_the_release_copies_them() {
    let mut tui = pane_with("hello world\r\nsecond line");
    select(&mut tui, (6, 0), (5, 1));

    assert_eq!(last_copy(&mut tui).as_deref(), Some("world\nsecond"));
}

#[test]
fn a_drag_backwards_selects_the_same_characters() {
    let mut tui = pane_with("hello world\r\nsecond line");
    select(&mut tui, (5, 1), (6, 0));

    assert_eq!(last_copy(&mut tui).as_deref(), Some("world\nsecond"));
}

#[test]
fn a_plain_click_copies_nothing_and_clears_the_selection() {
    let mut tui = pane_with("hello world");
    select(&mut tui, (0, 0), (4, 0));
    tui.take_effects();
    tui.later(1000);
    click(&mut tui, 8, 0);

    assert_eq!(copies(&mut tui), Vec::<String>::new());
    assert_eq!(tui.background_of("hello"), Color::Reset);
}

#[test]
fn the_selection_stays_highlighted_after_release_in_the_visual_colour() {
    let mut tui = pane_with("hello world");
    select(&mut tui, (0, 0), (4, 0));

    assert_eq!(tui.background_of("hello"), visual_colour());
    assert_eq!(tui.background_of("world"), Color::Reset);
}

#[test]
fn a_double_click_selects_a_word_keeping_a_url_as_one_unit() {
    let mut tui = pane_with("see https://example.com/a/b?c=1 now");
    click(&mut tui, 10, 0);
    click(&mut tui, 10, 0);

    assert_eq!(
        last_copy(&mut tui).as_deref(),
        Some("https://example.com/a/b?c=1")
    );
}

#[test]
fn a_triple_click_selects_the_line() {
    let mut tui = pane_with("  first line  \r\nsecond");
    click(&mut tui, 4, 0);
    click(&mut tui, 4, 0);
    click(&mut tui, 4, 0);

    assert_eq!(last_copy(&mut tui).as_deref(), Some("  first line"));
}

#[test]
fn dragging_after_a_double_click_extends_by_whole_words() {
    let mut tui = pane_with("alpha beta gamma delta");
    click(&mut tui, 7, 0);
    down(&mut tui, 7, 0);
    drag(&mut tui, 12, 0);
    up(&mut tui, 12, 0);

    assert_eq!(last_copy(&mut tui).as_deref(), Some("beta gamma"));
}

#[test]
fn dragging_after_a_triple_click_extends_by_whole_lines() {
    let mut tui = pane_with("one\r\ntwo\r\nthree");
    click(&mut tui, 1, 0);
    click(&mut tui, 1, 0);
    down(&mut tui, 1, 0);
    drag(&mut tui, 1, 1);
    up(&mut tui, 1, 1);

    assert_eq!(last_copy(&mut tui).as_deref(), Some("one\ntwo"));
}

#[test]
fn clicks_further_apart_than_400ms_are_not_a_double_click() {
    let mut tui = pane_with("alpha beta");
    click(&mut tui, 7, 0);
    tui.later(450);
    click(&mut tui, 7, 0);

    assert_eq!(copies(&mut tui), Vec::<String>::new());
}

#[test]
fn clicks_on_different_cells_are_not_a_double_click() {
    let mut tui = pane_with("alpha beta");
    click(&mut tui, 7, 0);
    click(&mut tui, 8, 0);

    assert_eq!(copies(&mut tui), Vec::<String>::new());
}

#[test]
fn the_wheel_scrolls_the_scrollback_by_three_lines() {
    let mut tui = long_history();
    wheel(&mut tui, MouseEventKind::ScrollUp, 1);
    assert!(visible(&mut tui, "line 071"));
    assert!(!visible(&mut tui, "line 070"));
    assert!(!visible(&mut tui, "line 098"));

    wheel(&mut tui, MouseEventKind::ScrollDown, 1);
    assert!(visible(&mut tui, "line 100"));
    assert!(!visible(&mut tui, "line 073"));
}

#[test]
fn the_wheel_over_the_sidebar_does_not_scroll_the_pane() {
    let mut tui = long_history();
    tui.wheel(MouseEventKind::ScrollUp, 10, 5);

    assert!(visible(&mut tui, "line 100"));
}

#[test]
fn a_selection_spans_scrollback_scrolled_with_the_wheel() {
    let mut tui = long_history();
    wheel(&mut tui, MouseEventKind::ScrollUp, 10);
    down(&mut tui, 0, 0);
    wheel(&mut tui, MouseEventKind::ScrollDown, 10);
    drag(&mut tui, 7, 26);
    up(&mut tui, 7, 26);

    assert_eq!(
        last_copy(&mut tui).as_deref(),
        Some(lines(44, 100).as_str())
    );
}

#[test]
fn a_drag_past_the_top_edge_auto_scrolls_the_scrollback() {
    let mut tui = long_history();
    down(&mut tui, 7, 26);
    for _ in 0..10 {
        tui.drag(PANE_LEFT, PANE_TOP - 1);
    }
    assert!(visible(&mut tui, "line 064"));
    tui.release(PANE_LEFT, PANE_TOP - 1);

    assert_eq!(
        last_copy(&mut tui).as_deref(),
        Some(lines(64, 100).as_str())
    );
}

#[test]
fn a_drag_past_the_bottom_edge_auto_scrolls_towards_the_live_screen() {
    let mut tui = long_history();
    wheel(&mut tui, MouseEventKind::ScrollUp, 10);
    down(&mut tui, 0, 0);
    for _ in 0..5 {
        tui.drag(PANE_LEFT + 7, BELOW_PANE);
    }
    tui.release(PANE_LEFT + 7, BELOW_PANE);

    assert_eq!(last_copy(&mut tui).as_deref(), Some(lines(44, 75).as_str()));
}

#[test]
fn auto_scroll_continues_on_ticks_while_the_pointer_rests_past_the_edge() {
    let mut tui = long_history();
    down(&mut tui, 7, 26);
    tui.drag(PANE_LEFT, PANE_TOP - 1);
    for _ in 0..5 {
        tui.tick();
    }
    tui.release(PANE_LEFT, PANE_TOP - 1);
    tui.tick();

    assert_eq!(
        last_copy(&mut tui).as_deref(),
        Some(lines(68, 100).as_str())
    );
    assert!(visible(&mut tui, "line 068"));
    assert!(!visible(&mut tui, "line 067"));
}

#[test]
fn ticks_do_not_scroll_once_the_pointer_is_back_inside_the_pane() {
    let mut tui = long_history();
    down(&mut tui, 7, 26);
    tui.drag(PANE_LEFT, PANE_TOP - 1);
    drag(&mut tui, 0, 0);
    tui.tick();

    assert!(visible(&mut tui, "line 073"));
    assert!(!visible(&mut tui, "line 072"));
}

#[test]
fn wrapped_rows_join_without_a_newline_and_trailing_blanks_are_trimmed() {
    let long = "0123456789".repeat(8);
    let mut tui = pane_with(&format!("abc   \r\n{long}\r\nend"));
    select(&mut tui, (0, 0), (1, 2));

    assert_eq!(last_copy(&mut tui), Some(format!("abc\n{long}")));
}

#[test]
fn a_double_click_selects_a_word_that_wraps_across_rows() {
    let long = "0123456789".repeat(8);
    let mut tui = pane_with(&format!("x {long} y"));
    click(&mut tui, 1, 1);
    click(&mut tui, 1, 1);

    assert_eq!(last_copy(&mut tui), Some(long));
}

#[test]
fn a_triple_click_selects_the_whole_wrapped_line() {
    let long = "0123456789".repeat(8);
    let mut tui = pane_with(&format!("{long}\r\nnext"));
    click(&mut tui, 1, 1);
    click(&mut tui, 1, 1);
    click(&mut tui, 1, 1);

    assert_eq!(last_copy(&mut tui), Some(long));
}

#[test]
fn the_selection_stays_on_its_text_when_new_output_scrolls_the_screen() {
    let mut tui = long_history();
    select(&mut tui, (0, 26), (7, 26));
    output(&mut tui, "\r\nline 101\r\nline 102");

    assert_ne!(tui.background_of("line 100"), Color::Reset);
    assert_eq!(tui.background_of("line 102"), Color::Reset);
}

#[test]
fn there_is_no_selection_when_the_agent_has_the_mouse() {
    let mut tui = pane_with("\x1b[?1002h\x1b[?1006hhello world");
    select(&mut tui, (0, 0), (4, 0));

    assert_eq!(copies(&mut tui), Vec::<String>::new());
    assert!(!tui.daemon().input.is_empty());
    assert_eq!(tui.background_of("hello"), Color::Reset);
}

#[test]
fn the_wheel_is_passed_through_instead_of_scrolling_when_the_agent_has_the_mouse() {
    let mut tui = long_history();
    output(&mut tui, "\x1b[?1000h\x1b[?1006h");
    wheel(&mut tui, MouseEventKind::ScrollUp, 1);

    assert!(visible(&mut tui, "line 100"));
    assert_eq!(tui.daemon().input, b"\x1b[<64;6;6M");
}

#[test]
fn selecting_in_insert_mode_copies_and_stays_in_insert_mode() {
    let mut tui = pane_with("hello world");
    tui.keys("i");
    select(&mut tui, (0, 0), (4, 0));

    assert_eq!(last_copy(&mut tui).as_deref(), Some("hello"));
    assert!(statusline(&mut tui).contains("INSERT"));
    assert!(tui.daemon().input.is_empty());
}

#[test]
fn entering_visual_mode_clears_the_mouse_selection() {
    let mut tui = pane_with("hello world\r\nnext");
    select(&mut tui, (6, 0), (10, 0));
    tui.keys("v");

    assert_eq!(tui.background_of("world"), Color::Reset);
}

#[test]
fn a_press_in_visual_mode_leaves_it_and_starts_a_mouse_selection() {
    let mut tui = pane_with("hello world\r\nnext");
    tui.keys("V");
    select(&mut tui, (0, 0), (4, 0));

    assert!(statusline(&mut tui).contains("NORMAL"));
    assert_eq!(last_copy(&mut tui).as_deref(), Some("hello"));
    assert_eq!(tui.background_of("next"), Color::Reset);
}

#[test]
fn nothing_is_selected_once_the_pane_has_closed() {
    let mut tui = pane_with("hello world");
    tui.pane(
        "first",
        FromDaemon::PaneClosed {
            reason: "gone".into(),
        },
    );
    select(&mut tui, (0, 0), (4, 0));

    assert_eq!(copies(&mut tui), Vec::<String>::new());
}

#[test]
fn a_double_click_on_wide_characters_copies_them_without_padding() {
    let mut tui = pane_with("say 日本語 now");
    click(&mut tui, 6, 0);
    click(&mut tui, 6, 0);

    assert_eq!(last_copy(&mut tui).as_deref(), Some("日本語"));
}

fn full_history() -> Harness {
    let text: Vec<String> = (1..=10_100).map(|n| format!("line {n:05}")).collect();
    pane_with(&text.join("\r\n"))
}

#[test]
fn a_selection_never_lands_on_other_text_once_the_scrollback_is_full() {
    let mut tui = full_history();
    select(&mut tui, (0, 26), (9, 26));
    output(&mut tui, "\r\nnext 1\r\nnext 2");

    assert_eq!(tui.background_of("next 2"), Color::Reset);
    assert_eq!(tui.background_of("line 10098"), Color::Reset);
}

#[test]
fn a_selection_survives_output_that_does_not_scroll_a_full_scrollback() {
    let mut tui = full_history();
    select(&mut tui, (0, 26), (9, 26));
    output(&mut tui, "\x1b[1;1H");

    assert_ne!(tui.background_of("line 10100"), Color::Reset);
}

#[test]
fn losing_terminal_focus_ends_the_selection_gesture() {
    let mut tui = long_history();
    down(&mut tui, 7, 26);
    tui.drag(PANE_LEFT, PANE_TOP - 1);
    tui.send(orch_tui::Event::Terminal(
        crossterm::event::Event::FocusLost,
    ));
    tui.tick();
    up(&mut tui, 0, 0);

    assert!(visible(&mut tui, "line 073"));
    assert!(!visible(&mut tui, "line 072"));
    assert_eq!(copies(&mut tui), Vec::<String>::new());
}

#[test]
fn a_selection_copy_under_wayland_sets_the_clipboard_and_primary_with_wl_copy() {
    let mut tui = Harness::with_config(TuiConfig {
        display: DisplayVars {
            wayland_display: Some("wayland-0".into()),
            x11_display: None,
        },
        ..TuiConfig::default()
    });
    tui.sessions(vec![session("webshop", "first")]);
    output(&mut tui, "hello world");
    tui.take_effects();
    select(&mut tui, (0, 0), (4, 0));

    let copy = |args: &[&str]| Effect::CopyCommand {
        program: "wl-copy".into(),
        args: args.iter().map(|arg| arg.to_string()).collect(),
        text: "hello".into(),
    };
    assert_eq!(tui.take_effects(), vec![copy(&[]), copy(&["--primary"])]);
}

#[test]
fn auto_scroll_asks_for_ticks_only_while_the_scrollback_can_move() {
    let mut tui = long_history();
    down(&mut tui, 7, 26);
    tui.drag(PANE_LEFT, PANE_TOP - 1);
    assert!(tui.tui.auto_scrolling());

    wheel(&mut tui, MouseEventKind::ScrollUp, 30);
    tui.tick();
    assert!(!tui.tui.auto_scrolling());
}

#[test]
fn a_drag_past_the_bottom_of_the_live_screen_asks_for_no_ticks() {
    let mut tui = long_history();
    down(&mut tui, 0, 20);
    tui.drag(PANE_LEFT, BELOW_PANE);

    assert!(!tui.tui.auto_scrolling());
}
