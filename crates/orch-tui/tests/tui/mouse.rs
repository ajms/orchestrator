use crossterm::event::{Event as TermEvent, KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use orch_protocol::{FromDaemon, Size};
use orch_tui::Event;

use crate::common::*;

const LEFT: MouseEventKind = MouseEventKind::Down(MouseButton::Left);
const MOVED: MouseEventKind = MouseEventKind::Moved;
const PANE: Size = Size { rows: 27, cols: 78 };

fn agent_with(modes: &str) -> Harness {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first")]);
    output(&mut tui, modes);
    tui
}

fn output(tui: &mut Harness, text: &str) {
    let bytes = text.as_bytes().to_vec();
    tui.pane("first", FromDaemon::Output { bytes });
}

fn forwarded(tui: &mut Harness) -> Vec<u8> {
    std::mem::take(&mut tui.daemon().input)
}

fn sent(tui: &mut Harness) -> String {
    String::from_utf8_lossy(&forwarded(tui)).into_owned()
}

fn click_pane(tui: &mut Harness, col: u16, row: u16) {
    let (x, y) = in_pane(col, row);
    tui.click(x, y);
}

fn at_pane(tui: &mut Harness, kind: MouseEventKind, col: u16, row: u16, mods: KeyModifiers) {
    let (x, y) = in_pane(col, row);
    tui.mouse(kind, x, y, mods);
}

fn focus(tui: &mut Harness, gained: bool) {
    let event = match gained {
        true => TermEvent::FocusGained,
        false => TermEvent::FocusLost,
    };
    tui.send(Event::Terminal(event));
}

#[test]
fn nothing_is_forwarded_when_the_agent_has_not_asked_for_the_mouse() {
    let mut tui = agent_with("hello");
    click_pane(&mut tui, 3, 3);
    let (x, y) = in_pane(3, 3);
    tui.wheel(MouseEventKind::ScrollUp, x, y);
    tui.mouse(MOVED, x, y, KeyModifiers::NONE);

    assert_eq!(sent(&mut tui), "");
}

#[test]
fn a_click_is_forwarded_as_sgr_relative_to_the_pane() {
    let mut tui = agent_with("\x1b[?1000h\x1b[?1006h");
    click_pane(&mut tui, 0, 0);
    click_pane(&mut tui, 4, 2);

    assert_eq!(
        sent(&mut tui),
        "\x1b[<0;1;1M\x1b[<0;1;1m\x1b[<0;5;3M\x1b[<0;5;3m"
    );
}

#[test]
fn middle_and_right_buttons_have_their_own_codes() {
    let mut tui = agent_with("\x1b[?1000h\x1b[?1006h");
    let none = KeyModifiers::NONE;
    at_pane(
        &mut tui,
        MouseEventKind::Down(MouseButton::Middle),
        0,
        0,
        none,
    );
    at_pane(
        &mut tui,
        MouseEventKind::Up(MouseButton::Middle),
        0,
        0,
        none,
    );
    at_pane(
        &mut tui,
        MouseEventKind::Down(MouseButton::Right),
        0,
        0,
        none,
    );
    at_pane(&mut tui, MouseEventKind::Up(MouseButton::Right), 0, 0, none);

    assert_eq!(
        sent(&mut tui),
        "\x1b[<1;1;1M\x1b[<1;1;1m\x1b[<2;1;1M\x1b[<2;1;1m"
    );
}

#[test]
fn press_mode_forwards_presses_and_the_wheel_only() {
    let mut tui = agent_with("\x1b[?9h\x1b[?1006h");
    let (x, y) = in_pane(1, 1);
    tui.mouse_down(x, y);
    tui.drag(x + 1, y);
    tui.release(x + 1, y);
    tui.mouse(MOVED, x, y, KeyModifiers::NONE);
    tui.wheel(MouseEventKind::ScrollUp, x, y);

    assert_eq!(sent(&mut tui), "\x1b[<0;2;2M\x1b[<64;2;2M");
}

#[test]
fn press_release_mode_adds_releases_but_not_drags() {
    let mut tui = agent_with("\x1b[?1000h\x1b[?1006h");
    let (x, y) = in_pane(1, 1);
    tui.mouse_down(x, y);
    tui.drag(x + 1, y);
    tui.release(x + 1, y);
    tui.mouse(MOVED, x, y, KeyModifiers::NONE);

    assert_eq!(sent(&mut tui), "\x1b[<0;2;2M\x1b[<0;3;2m");
}

#[test]
fn button_motion_mode_adds_drags_but_not_bare_motion() {
    let mut tui = agent_with("\x1b[?1002h\x1b[?1006h");
    let (x, y) = in_pane(1, 1);
    tui.mouse_down(x, y);
    tui.drag(x + 1, y);
    tui.release(x + 1, y);
    tui.mouse(MOVED, x, y, KeyModifiers::NONE);

    assert_eq!(sent(&mut tui), "\x1b[<0;2;2M\x1b[<32;3;2M\x1b[<0;3;2m");
}

#[test]
fn any_motion_mode_adds_bare_motion() {
    let mut tui = agent_with("\x1b[?1003h\x1b[?1006h");
    at_pane(&mut tui, MOVED, 5, 6, KeyModifiers::NONE);

    assert_eq!(sent(&mut tui), "\x1b[<35;6;7M");
}

#[test]
fn the_wheel_is_forwarded_in_every_direction() {
    let mut tui = agent_with("\x1b[?1000h\x1b[?1006h");
    let (x, y) = in_pane(0, 0);
    tui.wheel(MouseEventKind::ScrollUp, x, y);
    tui.wheel(MouseEventKind::ScrollDown, x, y);
    tui.wheel(MouseEventKind::ScrollLeft, x, y);
    tui.wheel(MouseEventKind::ScrollRight, x, y);

    assert_eq!(
        sent(&mut tui),
        "\x1b[<64;1;1M\x1b[<65;1;1M\x1b[<66;1;1M\x1b[<67;1;1M"
    );
}

#[test]
fn shift_alt_and_ctrl_set_the_modifier_bits() {
    let mut tui = agent_with("\x1b[?1000h\x1b[?1006h");
    at_pane(&mut tui, LEFT, 0, 0, KeyModifiers::SHIFT);
    at_pane(&mut tui, LEFT, 0, 0, KeyModifiers::ALT);
    at_pane(&mut tui, LEFT, 0, 0, KeyModifiers::CONTROL);
    let all = KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL;
    at_pane(&mut tui, LEFT, 0, 0, all);
    at_pane(
        &mut tui,
        MouseEventKind::ScrollUp,
        0,
        0,
        KeyModifiers::CONTROL,
    );

    assert_eq!(
        sent(&mut tui),
        "\x1b[<4;1;1M\x1b[<8;1;1M\x1b[<16;1;1M\x1b[<28;1;1M\x1b[<80;1;1M"
    );
}

#[test]
fn the_default_encoding_offsets_by_32_and_releases_as_button_3() {
    let mut tui = agent_with("\x1b[?1002h");
    let (x, y) = in_pane(4, 2);
    tui.mouse_down(x, y);
    tui.drag(x + 1, y);
    tui.release(x + 1, y);
    at_pane(
        &mut tui,
        MouseEventKind::ScrollDown,
        0,
        0,
        KeyModifiers::CONTROL,
    );

    assert_eq!(
        forwarded(&mut tui),
        [
            b"\x1b[M\x20\x25\x23".as_slice(),
            b"\x1b[M\x40\x26\x23",
            b"\x1b[M\x23\x26\x23",
            b"\x1b[M\x71\x21\x21",
        ]
        .concat()
    );
}

#[test]
fn the_default_encoding_drops_events_beyond_its_coordinate_limit() {
    let mut tui = agent_with("\x1b[?1000h");
    tui.resize(300, 30);
    click_pane(&mut tui, 223, 0);
    click_pane(&mut tui, 222, 0);

    assert_eq!(forwarded(&mut tui), b"\x1b[M\x20\xff\x21\x1b[M\x23\xff\x21");
}

#[test]
fn the_utf8_encoding_writes_large_coordinates_as_two_bytes() {
    let mut tui = agent_with("\x1b[?1000h\x1b[?1005h");
    tui.resize(200, 30);
    at_pane(&mut tui, LEFT, 99, 0, KeyModifiers::NONE);

    assert_eq!(forwarded(&mut tui), "\x1b[M \u{84}!".as_bytes());
}

#[test]
fn a_drag_that_wanders_onto_the_sidebar_stays_with_the_pane_clamped_to_its_edge() {
    let mut tui = agent_with("\x1b[?1002h\x1b[?1006h");
    let (x, y) = in_pane(3, 4);
    tui.mouse_down(x, y);
    tui.drag(10, y);
    tui.drag(10, HEIGHT - 1);
    tui.release(10, HEIGHT - 1);

    assert_eq!(
        sent(&mut tui),
        "\x1b[<0;4;5M\x1b[<32;1;5M\x1b[<32;1;27M\x1b[<0;1;27m"
    );
}

#[test]
fn a_gesture_that_starts_on_the_sidebar_is_not_forwarded_when_it_enters_the_pane() {
    let mut tui = agent_with("\x1b[?1002h\x1b[?1006h");
    tui.mouse_down(10, 3);
    let (x, y) = in_pane(3, 3);
    tui.drag(x, y);
    tui.release(x, y);

    assert_eq!(sent(&mut tui), "");
}

#[test]
fn the_wheel_and_bare_motion_off_the_pane_are_not_forwarded() {
    let mut tui = agent_with("\x1b[?1003h\x1b[?1006h");
    tui.wheel(MouseEventKind::ScrollUp, 10, 3);
    tui.mouse(MOVED, 10, 3, KeyModifiers::NONE);
    tui.wheel(MouseEventKind::ScrollUp, SIDEBAR, 3);
    tui.mouse(MOVED, 60, HEIGHT - 1, KeyModifiers::NONE);

    assert_eq!(sent(&mut tui), "");
}

#[test]
fn a_press_on_the_pane_border_is_not_forwarded() {
    let mut tui = agent_with("\x1b[?1000h\x1b[?1006h");
    tui.click(SIDEBAR, 5);
    tui.click(60, 0);

    assert_eq!(sent(&mut tui), "");
}

#[test]
fn nothing_is_forwarded_once_the_agent_releases_the_mouse() {
    let mut tui = agent_with("\x1b[?1000h\x1b[?1006h");
    output(&mut tui, "\x1b[?1000l");
    click_pane(&mut tui, 1, 1);

    assert_eq!(sent(&mut tui), "");
}

#[test]
fn mouse_modes_from_the_screen_snapshot_are_honoured() {
    let snapshot = screen_of("\x1b[?1002h\x1b[?1006h", PANE);
    let mut tui = Harness::with_screens(vec![(id("first"), snapshot)]);
    tui.sessions(vec![session("webshop", "first")]);
    click_pane(&mut tui, 0, 0);

    assert_eq!(sent(&mut tui), "\x1b[<0;1;1M\x1b[<0;1;1m");
}

#[test]
fn the_mouse_does_not_change_normal_mode() {
    let mut tui = agent_with("\x1b[?1000h\x1b[?1006h");
    click_pane(&mut tui, 1, 1);

    assert_eq!(sent(&mut tui), "\x1b[<0;2;2M\x1b[<0;2;2m");
    assert!(statusline(&mut tui).contains("NORMAL"));
}

#[test]
fn the_mouse_does_not_change_insert_mode() {
    let mut tui = agent_with("\x1b[?1000h\x1b[?1006h");
    tui.keys("i");
    click_pane(&mut tui, 1, 1);
    tui.wheel(MouseEventKind::ScrollUp, 10, 3);

    assert_eq!(sent(&mut tui), "\x1b[<0;2;2M\x1b[<0;2;2m");
    assert!(statusline(&mut tui).contains("INSERT"));
}

#[test]
fn the_mouse_is_not_forwarded_while_a_popup_is_open() {
    let mut tui = agent_with("\x1b[?1000h\x1b[?1006h");
    tui.command("new");
    click_pane(&mut tui, 1, 1);

    assert_eq!(sent(&mut tui), "");
}

#[test]
fn focus_changes_reach_an_agent_that_asked_for_them() {
    let mut tui = agent_with("\x1b[?1004h");
    sent(&mut tui);
    focus(&mut tui, false);
    focus(&mut tui, true);

    assert_eq!(sent(&mut tui), "\x1b[O\x1b[I");
}

#[test]
fn focus_changes_are_not_sent_to_an_agent_that_did_not_ask() {
    let mut tui = agent_with("hello");
    focus(&mut tui, false);
    focus(&mut tui, true);

    assert_eq!(sent(&mut tui), "");
}

#[test]
fn focus_changes_stop_once_the_agent_turns_focus_reporting_off() {
    let mut tui = agent_with("\x1b[?1004h");
    output(&mut tui, "\x1b[?2004;1004l");
    sent(&mut tui);
    focus(&mut tui, false);

    assert_eq!(sent(&mut tui), "");
}

#[test]
fn a_focus_request_split_across_output_chunks_is_recognised() {
    let mut tui = agent_with("\x1b[?10");
    output(&mut tui, "04h");
    focus(&mut tui, false);

    assert_eq!(sent(&mut tui), "\x1b[I\x1b[O");
}

#[test]
fn a_focus_request_carried_by_the_screen_snapshot_is_honoured() {
    let mut snapshot = screen_of("", PANE);
    snapshot.input_modes.extend_from_slice(b"\x1b[?1004h");
    let mut tui = Harness::with_screens(vec![(id("first"), snapshot)]);
    tui.sessions(vec![session("webshop", "first")]);
    focus(&mut tui, false);

    assert_eq!(sent(&mut tui), "\x1b[I\x1b[O");
}

#[test]
fn an_agent_that_asks_for_focus_while_shown_in_a_focused_terminal_is_told_once() {
    let mut tui = agent_with("\x1b[?1004h");
    output(&mut tui, "more output\x1b[?1004h");

    assert_eq!(sent(&mut tui), "\x1b[I");
}

#[test]
fn an_agent_shown_in_an_unfocused_terminal_is_told_only_when_focus_arrives() {
    let mut tui = Harness::unfocused();
    tui.sessions(vec![session("webshop", "first")]);
    output(&mut tui, "\x1b[?1004h");
    assert_eq!(sent(&mut tui), "");

    focus(&mut tui, true);
    assert_eq!(sent(&mut tui), "\x1b[I");
}

fn two_agents() -> Harness {
    let mut tui = Harness::new();
    tui.sessions(vec![
        session("webshop", "first"),
        session("webshop", "second"),
    ]);
    output(&mut tui, "\x1b[?1004h");
    tui
}

#[test]
fn the_agent_being_left_is_told_it_lost_focus_before_its_pane_closes() {
    let mut tui = two_agents();
    tui.keys("j");

    assert_eq!(tui.daemon().input_to("first"), "\x1b[I\x1b[O");
}

#[test]
fn the_newly_shown_agent_is_told_it_is_focused_once_it_asks() {
    let mut tui = two_agents();
    tui.keys("j");
    assert_eq!(tui.daemon().input_to("second"), "");

    let bytes = b"\x1b[?1004h".to_vec();
    tui.pane("second", FromDaemon::Output { bytes });
    assert_eq!(tui.daemon().input_to("second"), "\x1b[I");
}

#[test]
fn a_newly_shown_agent_whose_snapshot_asks_for_focus_is_told_it_is_focused() {
    let mut snapshot = screen_of("", PANE);
    snapshot.input_modes.extend_from_slice(b"\x1b[?1004h");
    let mut tui = Harness::with_screens(vec![(id("second"), snapshot)]);
    tui.sessions(vec![
        session("webshop", "first"),
        session("webshop", "second"),
    ]);
    tui.keys("j");

    assert_eq!(tui.daemon().input_to("second"), "\x1b[I");
}

#[test]
fn x10_mode_carries_no_modifier_bits() {
    let mut tui = agent_with("\x1b[?9h\x1b[?1006h");
    let all = KeyModifiers::SHIFT | KeyModifiers::ALT | KeyModifiers::CONTROL;
    at_pane(&mut tui, LEFT, 0, 0, all);

    assert_eq!(sent(&mut tui), "\x1b[<0;1;1M");
}

#[test]
fn the_utf8_encoding_drops_events_beyond_its_coordinate_limit() {
    let mut tui = agent_with("\x1b[?1000h\x1b[?1005h");
    tui.resize(2100, 30);
    at_pane(&mut tui, LEFT, 2015, 0, KeyModifiers::NONE);
    at_pane(&mut tui, LEFT, 2014, 0, KeyModifiers::NONE);

    assert_eq!(forwarded(&mut tui), "\x1b[M \u{7ff}!".as_bytes());
}

#[test]
fn losing_terminal_focus_ends_the_gesture_in_flight() {
    let mut tui = agent_with("\x1b[?1002h\x1b[?1006h");
    let (x, y) = in_pane(3, 3);
    tui.mouse_down(x, y);
    focus(&mut tui, false);
    sent(&mut tui);
    tui.drag(10, 3);

    assert_eq!(sent(&mut tui), "");
}

#[test]
fn suspending_for_the_editor_ends_the_gesture_in_flight() {
    let mut tui = agent_with("\x1b[?1002h\x1b[?1006h");
    let (x, y) = in_pane(3, 3);
    tui.mouse_down(x, y);
    tui.command("new");
    tui.ctrl('g');
    tui.press(KeyCode::Esc);
    sent(&mut tui);
    tui.drag(10, 3);

    assert_eq!(sent(&mut tui), "");
}
