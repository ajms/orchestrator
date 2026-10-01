use crossterm::event::{Event as TermEvent, KeyCode};
use orch_protocol::{FromDaemon, PhaseView, Size};
use ratatui::style::Color;

use crate::common::*;

const PANE: Size = Size {
    rows: HEIGHT - 3,
    cols: WIDTH - 40 - 2,
};

fn three_sessions() -> Harness {
    let mut tui = Harness::new();
    tui.sessions(vec![
        session("webshop", "first"),
        session("webshop", "second"),
        session("orchestrator", "third"),
    ]);
    tui
}

#[test]
fn the_first_session_is_selected_and_its_pane_opened_on_connect() {
    let mut tui = three_sessions();

    assert_eq!(tui.daemon().opened, vec![(id("first"), PANE)]);
    assert_eq!(tui.daemon().last_view(), Some((Some("first".into()), true)));
    assert_ne!(tui.sidebar_background_of("first"), Color::Reset);
}

#[test]
fn j_and_k_in_the_sidebar_switch_the_pane_and_report_the_view() {
    let mut tui = three_sessions();

    tui.keys("jj");
    assert_eq!(tui.daemon().open_panes(), vec!["first", "second", "third"]);
    assert_eq!(tui.daemon().last_view(), Some((Some("third".into()), true)));
    assert_ne!(tui.sidebar_background_of("third"), Color::Reset);
    assert_eq!(tui.sidebar_background_of("first"), Color::Reset);

    tui.keys("j");
    assert_eq!(tui.daemon().open_panes().len(), 3);

    tui.keys("k");
    assert_eq!(
        tui.daemon().last_view(),
        Some((Some("second".into()), true))
    );
    assert!(tui.daemon().closed >= 3);
}

#[test]
fn terminal_focus_changes_are_reported_with_the_visible_session() {
    let mut tui = three_sessions();

    tui.send(orch_tui::Event::Terminal(TermEvent::FocusLost));
    assert_eq!(
        tui.daemon().last_view(),
        Some((Some("first".into()), false))
    );
    tui.send(orch_tui::Event::Terminal(TermEvent::FocusGained));
    assert_eq!(tui.daemon().last_view(), Some((Some("first".into()), true)));
    let reports = tui.daemon().views().len();
    tui.send(orch_tui::Event::Terminal(TermEvent::FocusGained));
    assert_eq!(tui.daemon().views().len(), reports);
}

#[test]
fn the_pane_mirrors_the_snapshot_and_following_output() {
    let mut tui =
        Harness::with_screens(vec![(id("first"), screen_of("hello from the Agent", PANE))]);
    tui.sessions(vec![session("webshop", "first")]);
    assert!(tui.screen().contains("hello from the Agent"));

    tui.pane(
        "first",
        FromDaemon::Output {
            bytes: b"\r\nmore output".to_vec(),
        },
    );
    assert!(tui.screen().contains("more output"));
}

#[test]
fn output_for_a_pane_no_longer_shown_is_ignored() {
    let mut tui = three_sessions();
    tui.keys("j");

    tui.pane(
        "first",
        FromDaemon::Output {
            bytes: b"stale bytes".to_vec(),
        },
    );
    assert!(!tui.screen().contains("stale bytes"));
}

#[test]
fn the_pane_title_shows_the_session_preset_and_mode() {
    let mut view = session("webshop", "first");
    view.preset = "edits".into();
    view.mode = Some("acceptEdits".into());
    let mut tui = Harness::new();
    tui.sessions(vec![view]);

    assert!(tui.screen().contains("[edits · acceptEdits]"));
}

#[test]
fn the_pane_title_names_a_titled_session_by_its_title_and_slug() {
    let mut tui = Harness::new();
    tui.sessions(vec![titled(
        session("webshop", "fix-login"),
        "login redirect",
    )]);

    assert!(
        tui.screen()
            .contains("webshop / login redirect (fix-login)"),
        "{}",
        tui.screen()
    );
}

#[test]
fn a_session_without_a_live_agent_shows_its_setup_output_instead_of_a_pane() {
    let mut failed = in_phase(session("webshop", "broken"), PhaseView::SetupFailed);
    failed.setup_output = Some("npm ERR! could not resolve".into());
    let mut tui = Harness::new();
    tui.sessions(vec![
        failed,
        in_phase(session("webshop", "asleep"), PhaseView::Suspended),
    ]);

    assert!(tui.daemon().opened.is_empty());
    assert!(tui.screen().contains("npm ERR! could not resolve"));
    tui.keys("j");
    assert!(tui.daemon().opened.is_empty());
    assert!(tui.screen().contains(":resume"));
}

#[test]
fn a_closed_pane_shows_the_reason() {
    let mut tui = three_sessions();
    tui.pane(
        "first",
        FromDaemon::PaneClosed {
            reason: "the Holder went away".into(),
        },
    );
    assert!(tui.screen().contains("the Holder went away"));
}

#[test]
fn a_terminal_resize_resizes_the_open_pane() {
    let mut tui = three_sessions();
    tui.terminal.backend_mut().resize(100, 20);
    tui.send(orch_tui::Event::Terminal(TermEvent::Resize(100, 20)));

    assert_eq!(
        tui.daemon().resizes.last(),
        Some(&Size {
            rows: 17,
            cols: 100 - 40 - 2
        })
    );
    let _ = KeyCode::Null;
}

#[test]
fn the_pane_reopens_when_the_session_gets_a_new_holder() {
    let mut view = session("webshop", "first");
    view.holder_pid = Some(100);
    let mut tui = Harness::new();
    tui.sessions(vec![view.clone()]);
    tui.pane(
        "first",
        FromDaemon::PaneClosed {
            reason: "the Holder went away".into(),
        },
    );
    tui.changed(view.clone());
    assert_eq!(tui.daemon().open_panes(), vec!["first"]);

    view.holder_pid = Some(200);
    tui.changed(view);
    assert_eq!(tui.daemon().open_panes(), vec!["first", "first"]);
    assert!(!tui.screen().contains("the Holder went away"));
}

#[test]
fn the_terminal_counts_as_unfocused_until_it_reports_focus() {
    let mut tui = Harness::unfocused();
    tui.sessions(vec![session("webshop", "first")]);
    assert_eq!(
        tui.daemon().last_view(),
        Some((Some("first".into()), false))
    );

    tui.send(orch_tui::Event::Terminal(TermEvent::FocusGained));
    assert_eq!(tui.daemon().last_view(), Some((Some("first".into()), true)));
}

#[test]
fn late_output_from_a_replaced_pane_is_dropped() {
    let mut view = session("webshop", "first");
    view.holder_pid = Some(100);
    let mut tui = Harness::new();
    tui.sessions(vec![view.clone()]);
    let old = tui.daemon().pane_id("first");
    view.holder_pid = Some(200);
    tui.changed(view);

    tui.send(orch_tui::Event::Pane {
        pane: old,
        message: FromDaemon::Output {
            bytes: b"from the old Holder".to_vec(),
        },
    });
    assert!(!tui.screen().contains("from the old Holder"));
    tui.pane(
        "first",
        FromDaemon::Output {
            bytes: b"from the new Holder".to_vec(),
        },
    );
    assert!(tui.screen().contains("from the new Holder"));
}
