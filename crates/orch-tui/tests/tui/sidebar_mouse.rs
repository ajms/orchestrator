use crossterm::event::{KeyCode, MouseEventKind};
use orch_protocol::{AgentStateView, FromDaemon, PhaseView, Request, SubagentView};
use orch_tui::Event;
use ratatui::style::Color;

use crate::common::*;

fn row_of(tui: &mut Harness, needle: &str) -> u16 {
    let lines = tui.sidebar_lines();
    lines
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not in the sidebar:\n{}", lines.join("\n")))
        as u16
}

fn click_row(tui: &mut Harness, needle: &str) {
    let row = row_of(tui, needle);
    tui.click(5, row);
}

fn shown(tui: &mut Harness) -> Option<String> {
    tui.daemon().last_view().and_then(|(session, _)| session)
}

fn in_sidebar(tui: &mut Harness, needle: &str) -> bool {
    tui.sidebar_lines().iter().any(|line| line.contains(needle))
}

fn two_repos() -> Harness {
    let mut tui = Harness::new();
    tui.sessions(vec![
        session("webshop", "first"),
        session("webshop", "second"),
        session("orchestrator", "third"),
    ]);
    tui
}

fn many_sessions(count: usize) -> Harness {
    let mut tui = Harness::new();
    let sessions = (0..count)
        .map(|n| session("webshop", &format!("task-{n:02}")))
        .collect();
    tui.sessions(sessions);
    tui
}

#[test]
fn clicking_a_session_row_shows_that_session() {
    let mut tui = two_repos();

    click_row(&mut tui, "third");

    assert_eq!(tui.daemon().open_panes(), vec!["first", "third"]);
    assert_eq!(shown(&mut tui).as_deref(), Some("third"));
    assert_ne!(tui.sidebar_background_of("third"), Color::Reset);
}

#[test]
fn clicking_a_subagent_row_shows_its_parent_session() {
    let mut parent = with_agent(session("webshop", "parent"), AgentStateView::Working);
    parent.subagents = vec![SubagentView {
        id: "a".into(),
        agent_type: "Explore".into(),
        description: "find callers".into(),
        tool_count: 7,
        done: false,
    }];
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "sibling"), parent]);

    click_row(&mut tui, "find callers");

    assert_eq!(shown(&mut tui).as_deref(), Some("parent"));
}

#[test]
fn a_sidebar_click_focuses_the_sidebar() {
    let mut tui = two_repos();
    tui.press(KeyCode::Enter);

    click_row(&mut tui, "first");
    tui.keys("j");

    assert_eq!(shown(&mut tui).as_deref(), Some("second"));
}

#[test]
fn a_click_in_the_pane_or_on_its_border_focuses_the_pane() {
    let mut tui = two_repos();

    let (x, y) = in_pane(3, 3);
    tui.click(x, y);
    tui.keys("j");
    assert_eq!(shown(&mut tui).as_deref(), Some("first"));

    tui.keys("h");
    tui.click(SIDEBAR, 5);
    tui.keys("j");
    assert_eq!(shown(&mut tui).as_deref(), Some("first"));
}

#[test]
fn the_wheel_never_changes_focus() {
    let mut tui = two_repos();
    tui.press(KeyCode::Enter);

    tui.wheel(MouseEventKind::ScrollDown, 5, 3);
    tui.keys("j");

    assert_eq!(shown(&mut tui).as_deref(), Some("first"));
}

#[test]
fn a_click_in_normal_mode_never_enters_insert_mode() {
    let mut tui = two_repos();

    click_row(&mut tui, "second");

    assert!(statusline(&mut tui).contains("NORMAL"));
    tui.keys("x");
    assert!(tui.daemon().input.is_empty());
}

#[test]
fn a_click_in_insert_mode_stays_in_insert_mode_on_the_clicked_session() {
    let mut tui = two_repos();
    tui.keys("i");

    click_row(&mut tui, "third");

    assert_eq!(shown(&mut tui).as_deref(), Some("third"));
    assert!(statusline(&mut tui).contains("INSERT"));
    tui.keys("x");
    assert_eq!(tui.daemon().input_to("third"), "x");
}

#[test]
fn a_click_in_insert_mode_on_a_session_without_a_live_agent_drops_to_normal_mode() {
    let mut tui = Harness::new();
    tui.sessions(vec![
        session("webshop", "first"),
        in_phase(session("webshop", "asleep"), PhaseView::Suspended),
        with_agent(session("webshop", "finished"), AgentStateView::Exited),
    ]);

    for target in ["asleep", "finished"] {
        tui.keys("hk");
        tui.keys("i");
        click_row(&mut tui, target);

        assert_eq!(shown(&mut tui).as_deref(), Some(target));
        assert!(statusline(&mut tui).contains("NORMAL"), "{target}");
        tui.keys("x");
        assert!(tui.daemon().input.is_empty(), "{target}");
    }
    let resumed = tui
        .daemon()
        .requests()
        .into_iter()
        .any(|request| matches!(request, Request::Resume { .. }));
    assert!(!resumed);
}

#[test]
fn the_wheel_scrolls_a_long_sidebar_without_changing_the_selection() {
    let mut tui = many_sessions(40);
    assert!(in_sidebar(&mut tui, "task-00"));
    assert!(!in_sidebar(&mut tui, "task-39"));

    for _ in 0..10 {
        tui.wheel(MouseEventKind::ScrollDown, 5, 10);
    }

    assert!(!in_sidebar(&mut tui, "task-00"));
    assert!(in_sidebar(&mut tui, "task-39"));
    assert_eq!(tui.daemon().open_panes(), vec!["task-00"]);

    for _ in 0..10 {
        tui.wheel(MouseEventKind::ScrollUp, 5, 10);
    }
    assert!(in_sidebar(&mut tui, "task-00"));
}

#[test]
fn a_click_on_a_scrolled_sidebar_shows_the_row_under_the_pointer() {
    let mut tui = many_sessions(40);
    for _ in 0..10 {
        tui.wheel(MouseEventKind::ScrollDown, 5, 10);
    }

    click_row(&mut tui, "task-30");

    assert_eq!(shown(&mut tui).as_deref(), Some("task-30"));
}

#[test]
fn keyboard_selection_keeps_the_selected_row_visible() {
    let mut tui = many_sessions(40);

    tui.keys(&"j".repeat(35));
    assert_eq!(shown(&mut tui).as_deref(), Some("task-35"));
    assert!(in_sidebar(&mut tui, "task-35"));

    tui.keys(&"k".repeat(35));
    assert!(in_sidebar(&mut tui, "task-00"));
}

#[test]
fn clicking_a_repo_heading_folds_and_unfolds_its_group() {
    let mut tui = two_repos();

    click_row(&mut tui, "webshop");
    assert!(!in_sidebar(&mut tui, "first"));
    assert!(!in_sidebar(&mut tui, "second"));
    assert!(in_sidebar(&mut tui, "third"));
    assert!(tui.sidebar_line_with("webshop").contains("(2)"));

    click_row(&mut tui, "webshop");
    assert!(in_sidebar(&mut tui, "first"));
    assert!(in_sidebar(&mut tui, "second"));
    assert!(!tui.sidebar_line_with("webshop").contains("(2)"));
}

#[test]
fn za_on_a_session_folds_its_repo_and_selects_the_heading_instead() {
    let mut tui = two_repos();

    tui.keys("za");

    assert!(!in_sidebar(&mut tui, "first"));
    assert_ne!(tui.sidebar_background_of("webshop"), Color::Reset);
    assert_eq!(shown(&mut tui), None);
    assert!(tui.screen().contains("webshop is folded"));

    tui.keys("za");
    assert!(in_sidebar(&mut tui, "first"));
    assert_eq!(shown(&mut tui).as_deref(), Some("first"));
    assert_ne!(tui.sidebar_background_of("first"), Color::Reset);
}

#[test]
fn j_and_k_stop_on_a_folded_heading_and_enter_unfolds_it() {
    let mut tui = two_repos();
    click_row(&mut tui, "webshop");

    tui.keys("j");
    assert_eq!(shown(&mut tui).as_deref(), Some("third"));
    tui.keys("k");
    assert_eq!(shown(&mut tui), None);
    assert_ne!(tui.sidebar_background_of("webshop"), Color::Reset);

    tui.press(KeyCode::Enter);
    assert!(in_sidebar(&mut tui, "second"));
    assert_eq!(shown(&mut tui).as_deref(), Some("first"));
    tui.keys("j");
    assert_eq!(shown(&mut tui).as_deref(), Some("second"));
}

#[test]
fn stepping_over_a_folded_repo_never_views_its_sessions() {
    let mut unviewed = with_agent(session("webshop", "unviewed"), AgentStateView::Working);
    unviewed.flags.unseen = true;
    let mut tui = Harness::new();
    tui.sessions(vec![
        session("orchestrator", "top"),
        unviewed,
        session("zoo", "bottom"),
    ]);
    click_row(&mut tui, "webshop");

    tui.keys("jjkk");

    let viewed: Vec<_> = tui.daemon().views().into_iter().map(|(id, _)| id).collect();
    assert!(!viewed.contains(&Some("unviewed".into())), "{viewed:?}");
    assert!(!tui.daemon().open_panes().contains(&"unviewed".into()));
    assert!(tui.sidebar_line_with("webshop").contains('●'));
    assert_eq!(shown(&mut tui).as_deref(), Some("top"));
}

#[test]
fn a_session_focused_by_the_daemon_is_revealed_even_in_a_folded_repo() {
    let mut tui = many_sessions(40);
    tui.send(Event::Daemon(FromDaemon::Focus {
        session: id("task-35"),
    }));
    assert!(in_sidebar(&mut tui, "task-35"));

    let mut tui = two_repos();
    click_row(&mut tui, "orchestrator");
    tui.send(Event::Daemon(FromDaemon::Focus {
        session: id("third"),
    }));
    assert!(in_sidebar(&mut tui, "third"));
    assert_eq!(shown(&mut tui).as_deref(), Some("third"));
}

#[test]
fn the_next_session_is_revealed_when_the_selected_one_is_removed() {
    let mut tui = many_sessions(40);
    tui.keys(&"j".repeat(35));
    for _ in 0..10 {
        tui.wheel(MouseEventKind::ScrollUp, 5, 10);
    }
    assert!(!in_sidebar(&mut tui, "task-35"));

    tui.send(Event::Daemon(FromDaemon::SessionRemoved {
        session: id("task-35"),
    }));

    assert_eq!(shown(&mut tui).as_deref(), Some("task-36"));
    assert!(in_sidebar(&mut tui, "task-36"));
}

#[test]
fn a_heading_click_in_insert_mode_only_folds_and_stays_inserting() {
    let mut tui = two_repos();
    tui.keys("i");

    click_row(&mut tui, "orchestrator");

    assert!(!in_sidebar(&mut tui, "third"));
    assert_eq!(shown(&mut tui).as_deref(), Some("first"));
    assert!(statusline(&mut tui).contains("INSERT"));
    tui.keys("x");
    assert_eq!(tui.daemon().input_to("first"), "x");
}

#[test]
fn enter_on_a_session_row_still_focuses_the_pane() {
    let mut tui = two_repos();

    tui.press(KeyCode::Enter);
    tui.keys("j");

    assert!(in_sidebar(&mut tui, "second"));
    assert_eq!(shown(&mut tui).as_deref(), Some("first"));
}

#[test]
fn a_folded_heading_shows_needs_input_before_unseen() {
    let mut unseen = session("webshop", "unseen-one");
    unseen.flags.unseen = true;
    let mut quiet_unseen = session("orchestrator", "quiet");
    quiet_unseen.flags.unseen = true;
    let mut tui = Harness::new();
    tui.sessions(vec![
        unseen,
        with_agent(session("webshop", "asking"), AgentStateView::NeedsInput),
        session("webshop", "calm"),
        quiet_unseen,
        session("orchestrator", "other"),
    ]);

    click_row(&mut tui, "orchestrator");
    click_row(&mut tui, "webshop");

    let webshop = tui.sidebar_line_with("webshop");
    assert!(webshop.contains("(3)"), "{webshop}");
    assert!(webshop.contains("Needs input"), "{webshop}");
    assert_eq!(tui.sidebar_colour_of("Needs input"), Color::Yellow);
    let orchestrator = tui.sidebar_line_with("orchestrator");
    assert!(orchestrator.contains("(2)"), "{orchestrator}");
    assert!(orchestrator.contains('●'), "{orchestrator}");
    assert!(!orchestrator.contains("Needs input"), "{orchestrator}");
    assert_eq!(tui.sidebar_colour_of("●"), Color::Yellow);
}
