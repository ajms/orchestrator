use crossterm::event::{Event as TermEvent, KeyCode, KeyEvent, KeyModifiers};
use orch_protocol::{AgentStateView, PhaseView, Request, Size};
use orch_tui::Event;

use crate::common::*;

fn one_session() -> Harness {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first")]);
    tui
}

fn statusline(tui: &mut Harness) -> String {
    tui.lines().pop().unwrap()
}

#[test]
fn the_client_starts_in_normal_mode_and_i_enters_insert_mode() {
    let mut tui = one_session();
    assert!(statusline(&mut tui).contains("NORMAL"));

    tui.keys("i");
    assert!(statusline(&mut tui).contains("INSERT"));
    assert!(tui.daemon().input.is_empty());
}

#[test]
fn a_also_enters_insert_mode() {
    let mut tui = one_session();
    tui.keys("a");
    assert!(statusline(&mut tui).contains("INSERT"));
}

#[test]
fn insert_mode_sends_every_key_to_the_pane_including_esc() {
    let mut tui = one_session();
    tui.keys("i");
    tui.keys("ls -la\n");
    tui.press(KeyCode::Esc);
    tui.ctrl('c');
    tui.keys(":jk");

    assert_eq!(tui.daemon().input, b"ls -la\r\x1b\x03:jk");
    assert!(statusline(&mut tui).contains("INSERT"));
}

#[test]
fn ctrl_backslash_ctrl_n_returns_to_normal_mode() {
    let mut tui = one_session();
    tui.keys("i");
    tui.ctrl('\\');
    tui.ctrl('n');

    assert!(statusline(&mut tui).contains("NORMAL"));
    assert!(tui.daemon().input.is_empty());
}

#[test]
fn ctrl_4_is_accepted_as_ctrl_backslash() {
    let mut tui = one_session();
    tui.keys("i");
    tui.ctrl('4');
    tui.ctrl('n');

    assert!(statusline(&mut tui).contains("NORMAL"));
    assert!(tui.daemon().input.is_empty());
}

#[test]
fn ctrl_backslash_followed_by_another_key_sends_both() {
    let mut tui = one_session();
    tui.keys("i");
    tui.ctrl('\\');
    tui.keys("x");

    assert_eq!(tui.daemon().input, b"\x1cx");
    assert!(statusline(&mut tui).contains("INSERT"));
}

#[test]
fn arrow_keys_honour_the_panes_application_cursor_mode() {
    let size = Size { rows: 27, cols: 78 };
    let mut tui = Harness::with_screens(vec![(id("first"), screen_of("\x1b[?1h", size))]);
    tui.sessions(vec![session("webshop", "first")]);
    tui.keys("i");
    tui.press(KeyCode::Up);

    assert_eq!(tui.daemon().input, b"\x1bOA");
}

#[test]
fn a_paste_in_insert_mode_goes_to_the_pane_as_a_paste() {
    let mut tui = one_session();
    tui.keys("i");
    tui.send(Event::Terminal(TermEvent::Paste("two\nlines".into())));

    assert_eq!(tui.daemon().pastes, vec!["two\nlines".to_string()]);
}

#[test]
fn i_on_a_suspended_or_ended_agent_resumes_it_instead() {
    let mut tui = Harness::new();
    tui.sessions(vec![
        in_phase(session("webshop", "asleep"), PhaseView::Suspended),
        with_agent(session("webshop", "ended"), AgentStateView::Exited),
    ]);

    tui.keys("i");
    tui.keys("ja");
    let resumed: Vec<Request> = tui
        .daemon()
        .requests()
        .into_iter()
        .filter(|request| matches!(request, Request::Resume { .. }))
        .collect();
    assert_eq!(
        resumed,
        vec![
            Request::Resume {
                session: id("asleep")
            },
            Request::Resume {
                session: id("ended")
            },
        ]
    );
    assert!(statusline(&mut tui).contains("NORMAL"));
}

#[test]
fn i_while_setting_up_explains_there_is_no_agent_yet() {
    let mut tui = Harness::new();
    tui.sessions(vec![in_phase(
        session("webshop", "fresh"),
        PhaseView::SettingUp,
    )]);
    tui.keys("i");

    let status = statusline(&mut tui);
    assert!(status.contains("NORMAL"));
    assert!(status.contains("no live Agent"), "{status}");
}

#[test]
fn ctrl_w_moves_focus_between_sidebar_and_pane() {
    let mut tui = Harness::new();
    tui.sessions(vec![
        session("webshop", "first"),
        session("webshop", "second"),
    ]);

    tui.ctrl('w');
    tui.keys("l");
    tui.keys("j");
    assert_eq!(tui.daemon().open_panes(), vec!["first"]);

    tui.ctrl('w');
    tui.keys("h");
    tui.keys("j");
    assert_eq!(tui.daemon().open_panes(), vec!["first", "second"]);

    tui.ctrl('w');
    tui.keys("w");
    tui.keys("k");
    assert_eq!(tui.daemon().open_panes(), vec!["first", "second"]);
    tui.ctrl('w');
    tui.keys("w");
    tui.keys("k");
    assert_eq!(tui.daemon().open_panes(), vec!["first", "second", "first"]);
}

#[test]
fn enter_or_l_focuses_the_pane_and_h_returns_to_the_sidebar() {
    let mut tui = Harness::new();
    tui.sessions(vec![
        session("webshop", "first"),
        session("webshop", "second"),
    ]);

    tui.press(KeyCode::Enter);
    tui.keys("j");
    assert_eq!(tui.daemon().open_panes(), vec!["first"]);
    tui.keys("hj");
    assert_eq!(tui.daemon().open_panes(), vec!["first", "second"]);
    tui.keys("lk");
    assert_eq!(tui.daemon().open_panes(), vec!["first", "second"]);
    let _ = KeyEvent::new(KeyCode::Null, KeyModifiers::NONE);
}

#[test]
fn a_closed_pane_drops_back_to_normal_mode() {
    let mut tui = one_session();
    tui.keys("i");
    tui.pane(
        "first",
        orch_protocol::FromDaemon::PaneClosed {
            reason: "the Holder went away".into(),
        },
    );
    assert!(statusline(&mut tui).contains("NORMAL"));
    tui.keys("x");
    assert!(tui.daemon().input.is_empty());
}

#[test]
fn i_on_a_suspended_session_enters_insert_mode_once_its_pane_reopens() {
    let mut view = in_phase(session("webshop", "asleep"), PhaseView::Suspended);
    let mut tui = Harness::new();
    tui.sessions(vec![view.clone()]);
    tui.keys("i");
    assert!(statusline(&mut tui).contains("NORMAL"));

    view.phase = PhaseView::Active;
    view.agent = Some(AgentStateView::Starting);
    view.holder_pid = Some(7);
    tui.changed(view);
    assert!(statusline(&mut tui).contains("INSERT"));
    tui.keys("x");
    assert_eq!(tui.daemon().input, b"x");
}
