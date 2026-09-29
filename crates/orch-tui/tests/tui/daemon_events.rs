use orch_protocol::FromDaemon;
use orch_tui::Event;
use ratatui::style::Color;

use crate::common::*;

fn three() -> Harness {
    let mut tui = Harness::new();
    tui.sessions(vec![
        session("webshop", "first"),
        session("webshop", "second"),
        session("webshop", "third"),
    ]);
    tui
}

#[test]
fn a_notification_click_focuses_that_sessions_pane() {
    let mut tui = three();
    tui.send(Event::Daemon(FromDaemon::Focus {
        session: id("third"),
    }));

    assert_eq!(tui.daemon().last_view(), Some((Some("third".into()), true)));
    assert_eq!(
        tui.daemon().open_panes().last().map(String::as_str),
        Some("third")
    );
    tui.keys("k");
    assert_eq!(
        tui.daemon().open_panes().len(),
        2,
        "k scrolls the focused pane"
    );
}

#[test]
fn a_removed_session_leaves_the_sidebar_and_the_selection_moves_on() {
    let mut tui = three();
    tui.keys("j");
    tui.send(Event::Daemon(FromDaemon::SessionRemoved {
        session: id("second"),
    }));

    assert!(!tui.sidebar_lines().join("\n").contains("second"));
    assert_eq!(tui.daemon().last_view(), Some((Some("third".into()), true)));
    assert_eq!(
        tui.daemon().open_panes().last().map(String::as_str),
        Some("third")
    );
}

#[test]
fn removing_the_last_session_selects_the_one_before() {
    let mut tui = three();
    tui.keys("jj");
    tui.send(Event::Daemon(FromDaemon::SessionRemoved {
        session: id("third"),
    }));
    assert_eq!(
        tui.daemon().last_view(),
        Some((Some("second".into()), true))
    );
}

#[test]
fn a_session_of_a_missing_repo_is_greyed_and_the_repo_marked_missing() {
    let mut gone = session("vanished", "cut-off");
    gone.flags.repo_missing = true;
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "fine"), gone]);

    assert!(tui.sidebar_line_with("vanished").contains("missing"));
    assert_eq!(tui.sidebar_colour_of("cut-off"), Color::DarkGray);
    assert_ne!(tui.sidebar_colour_of("fine"), Color::DarkGray);
}

#[test]
fn a_notification_click_closes_popups_so_the_session_is_visible() {
    let mut tui = three();
    tui.command("new");
    assert!(tui.screen().contains(":new Session"));
    tui.send(Event::Daemon(FromDaemon::Focus {
        session: id("second"),
    }));
    assert!(!tui.screen().contains(":new Session"));
    tui.keys("i");
    assert!(statusline(&mut tui).contains("INSERT"));
}

#[test]
fn a_notification_click_keeps_that_sessions_guard_prompt() {
    let mut guarded = session("webshop", "second");
    guarded.guard_prompts = vec![orch_protocol::GuardPrompt {
        id: 1,
        tool: "Bash".into(),
        kind: orch_protocol::GuardKindView::OtherRef,
        target: "git push origin other".into(),
    }];
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first"), guarded]);
    tui.command("new");
    tui.send(Event::Daemon(FromDaemon::Focus {
        session: id("second"),
    }));
    let screen = tui.screen();
    assert!(!screen.contains(":new Session"), "{screen}");
    assert!(screen.contains("git push origin other"), "{screen}");
}
