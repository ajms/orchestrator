use crossterm::event::KeyCode;
use orch_protocol::{AgentStateView, FromDaemon, SessionView};
use orch_tui::Event;
use ratatui::style::Color;

use crate::common::*;

fn parent(done: [bool; 3]) -> SessionView {
    let mut parent = with_agent(session("webshop", "parent"), AgentStateView::Working);
    parent.subagents = vec![
        subagent("a", "Explore", "find callers", done[0]),
        subagent("b", "Plan", "outline refactor", done[1]),
        subagent("c", "Explore", "read tests", done[2]),
    ];
    parent
}

fn family() -> Harness {
    let mut tui = Harness::new();
    tui.sessions(vec![
        session("webshop", "above"),
        parent([false, true, true]),
        session("webshop", "below"),
    ]);
    tui.keys("j");
    tui
}

fn highlighted(tui: &mut Harness, needle: &str) -> bool {
    tui.sidebar_background_of(needle) != Color::Reset
}

fn pane_shows(tui: &mut Harness, needle: &str) -> bool {
    tui.pane_lines().iter().any(|line| line.contains(needle))
}

#[test]
fn j_and_k_move_between_sessions_and_skip_subagent_rows() {
    let mut tui = family();

    tui.keys("j");
    assert_eq!(shown(&mut tui).as_deref(), Some("below"));
    assert!(!highlighted(&mut tui, "find callers"));
    assert!(!highlighted(&mut tui, "2 done"));

    tui.keys("k");
    assert_eq!(shown(&mut tui).as_deref(), Some("parent"));
    assert!(highlighted(&mut tui, "parent"));
}

#[test]
fn shift_j_and_k_step_through_the_subagents_and_stop_at_the_edges() {
    let mut tui = family();

    tui.keys("J");
    assert!(highlighted(&mut tui, "find callers"));
    assert!(!highlighted(&mut tui, "parent"));

    tui.keys("J");
    assert!(highlighted(&mut tui, "2 done"));
    assert!(!highlighted(&mut tui, "find callers"));

    tui.keys("J");
    assert!(highlighted(&mut tui, "2 done"));
    assert!(!highlighted(&mut tui, "below"));

    tui.keys("KK");
    assert!(highlighted(&mut tui, "parent"));
    tui.keys("K");
    assert!(highlighted(&mut tui, "parent"));
    assert!(!highlighted(&mut tui, "above"));
}

#[test]
fn j_from_a_subagent_goes_to_the_next_session_and_k_to_its_parent() {
    let mut tui = family();

    tui.keys("Jj");
    assert_eq!(shown(&mut tui).as_deref(), Some("below"));
    assert!(!highlighted(&mut tui, "find callers"));

    tui.keys("kJk");
    assert!(highlighted(&mut tui, "parent"));
    assert!(!highlighted(&mut tui, "find callers"));
}

#[test]
fn shift_j_on_a_session_without_subagents_stays_put() {
    let mut tui = family();

    tui.keys("jJ");

    assert!(highlighted(&mut tui, "below"));
}

#[test]
fn enter_on_the_done_group_expands_it_into_selectable_rows() {
    let mut tui = family();
    tui.keys("JJ");
    assert!(!in_sidebar(&mut tui, "outline refactor"));

    tui.press(KeyCode::Enter);
    assert!(in_sidebar(&mut tui, "outline refactor"));
    assert!(in_sidebar(&mut tui, "read tests"));
    assert!(highlighted(&mut tui, "2 done"));

    tui.keys("JJ");
    assert!(highlighted(&mut tui, "read tests"));
    assert!(!highlighted(&mut tui, "2 done"));

    tui.keys("KK");
    tui.press(KeyCode::Enter);
    assert!(!in_sidebar(&mut tui, "outline refactor"));
    assert!(highlighted(&mut tui, "2 done"));
}

#[test]
fn clicking_the_done_group_expands_it_and_only_enter_collapses_it() {
    let mut tui = family();

    click_row(&mut tui, "2 done");
    assert!(in_sidebar(&mut tui, "outline refactor"));
    assert!(highlighted(&mut tui, "2 done"));

    click_row(&mut tui, "outline refactor");
    assert!(highlighted(&mut tui, "outline refactor"));

    click_row(&mut tui, "2 done");
    assert!(in_sidebar(&mut tui, "outline refactor"));
    assert!(highlighted(&mut tui, "2 done"));

    tui.press(KeyCode::Enter);
    assert!(!in_sidebar(&mut tui, "outline refactor"));
}

#[test]
fn finished_subagents_are_ticked_and_dimmed() {
    let mut tui = family();

    click_row(&mut tui, "2 done");

    assert!(tui.sidebar_line_with("outline refactor").contains('✓'));
    assert!(!tui.sidebar_line_with("find callers").contains('✓'));
    assert_eq!(tui.sidebar_colour_of("outline refactor"), Color::DarkGray);
    assert_ne!(tui.sidebar_colour_of("find callers"), Color::DarkGray);
}

#[test]
fn ctrl_c_on_a_subagent_or_the_done_group_reaches_the_parent_agent() {
    let mut tui = family();

    tui.keys("J");
    tui.ctrl('c');
    tui.keys("J");
    tui.ctrl('c');

    assert_eq!(tui.daemon().input_to("parent"), "\x03\x03");
}

#[test]
fn removing_the_parent_of_a_selected_subagent_steps_to_the_next_session() {
    let mut tui = family();
    tui.keys("J");

    tui.send(Event::Daemon(FromDaemon::SessionRemoved {
        session: id("parent"),
    }));

    assert_eq!(shown(&mut tui).as_deref(), Some("below"));
    assert!(highlighted(&mut tui, "below"));
}

#[test]
fn the_done_group_expands_per_session() {
    let mut other = with_agent(session("webshop", "other"), AgentStateView::Working);
    other.subagents = vec![subagent("z", "Plan", "other plan", true)];
    let mut tui = Harness::new();
    tui.sessions(vec![parent([false, true, true]), other]);

    click_row(&mut tui, "2 done");

    assert!(in_sidebar(&mut tui, "outline refactor"));
    assert!(!in_sidebar(&mut tui, "other plan"));
}

#[test]
fn clicking_a_subagent_row_selects_that_subagent() {
    let mut tui = family();
    tui.keys("k");

    click_row(&mut tui, "find callers");

    assert!(highlighted(&mut tui, "find callers"));
    assert!(!highlighted(&mut tui, "above"));
    assert!(!highlighted(&mut tui, "parent"));
    assert_eq!(shown(&mut tui).as_deref(), Some("parent"));
    assert!(pane_shows(&mut tui, "Explore: find callers"));
}

#[test]
fn a_selected_subagent_shows_its_transcript_instead_of_the_agent_pane() {
    let mut tui = family();
    tui.keys("J");

    assert!(pane_shows(&mut tui, "Explore: find callers"));

    tui.keys("K");
    assert!(!pane_shows(&mut tui, "Explore: find callers"));
}

#[test]
fn a_selected_subagent_stays_selected_when_it_finishes() {
    let mut tui = family();
    tui.keys("J");

    tui.changed(parent([true, true, true]));

    assert!(in_sidebar(&mut tui, "3 done"));
    assert!(in_sidebar(&mut tui, "find callers"));
    assert!(highlighted(&mut tui, "find callers"));
    assert!(pane_shows(&mut tui, "Explore: find callers"));
}

#[test]
fn a_selected_subagent_that_disappears_hands_the_selection_to_its_parent() {
    let mut tui = family();
    tui.keys("J");

    tui.changed(with_agent(
        session("webshop", "parent"),
        AgentStateView::Working,
    ));

    assert!(highlighted(&mut tui, "parent"));
    assert_eq!(shown(&mut tui).as_deref(), Some("parent"));
}

#[test]
fn i_on_a_subagent_inserts_into_its_parent_session() {
    let mut tui = family();
    tui.keys("J");

    tui.keys("ix");

    assert!(statusline(&mut tui).contains("INSERT"));
    assert!(highlighted(&mut tui, "parent"));
    assert_eq!(tui.daemon().input_to("parent"), "x");
}

#[test]
fn a_click_on_a_subagent_in_insert_mode_drops_to_normal_mode() {
    let mut tui = family();
    tui.keys("i");

    click_row(&mut tui, "find callers");
    tui.keys("x");

    assert!(statusline(&mut tui).contains("NORMAL"));
    assert!(tui.daemon().input.is_empty());
}
