use orch_protocol::{AgentStateView, PhaseView, PrChecksView, PrReviewView, PrView, SubagentView};
use ratatui::style::Color;

use crate::common::*;

#[test]
fn the_sidebar_lists_sessions_grouped_by_repo() {
    let mut tui = Harness::new();
    tui.sessions(vec![
        session("webshop", "fix-login"),
        session("orchestrator", "daemon-socket"),
        session("webshop", "cart-refactor"),
    ]);

    let lines = tui.sidebar_lines();
    let row = |needle: &str| lines.iter().position(|line| line.contains(needle)).unwrap();
    assert!(row("webshop") < row("fix-login"));
    assert!(row("fix-login") < row("cart-refactor"));
    assert!(row("cart-refactor") < row("orchestrator"));
    assert!(row("orchestrator") < row("daemon-socket"));
}

#[test]
fn a_session_row_shows_its_agent_state_in_colour() {
    let mut tui = Harness::new();
    tui.sessions(vec![
        with_agent(session("webshop", "busy"), AgentStateView::Working),
        with_agent(session("webshop", "asking"), AgentStateView::NeedsInput),
        with_agent(session("webshop", "broken"), AgentStateView::Errored),
    ]);

    assert!(tui.sidebar_line_with("busy").contains("Working"));
    assert!(tui.sidebar_line_with("asking").contains("Needs input"));
    assert!(tui.sidebar_line_with("broken").contains("Errored"));
    assert_eq!(tui.colour_of("Working"), Color::Green);
    assert_eq!(tui.colour_of("Needs input"), Color::Yellow);
    assert_eq!(tui.colour_of("Errored"), Color::Red);
}

#[test]
fn a_session_without_a_live_agent_shows_its_phase() {
    let mut tui = Harness::new();
    tui.sessions(vec![
        in_phase(session("webshop", "preparing"), PhaseView::SettingUp),
        in_phase(session("webshop", "failed-setup"), PhaseView::SetupFailed),
        in_phase(session("webshop", "rebooted"), PhaseView::Suspended),
    ]);

    assert!(tui.sidebar_line_with("preparing").contains("Setting up"));
    assert!(
        tui.sidebar_line_with("failed-setup")
            .contains("Setup failed")
    );
    assert!(tui.sidebar_line_with("rebooted").contains("Suspended"));
    assert_eq!(tui.colour_of("Setup failed"), Color::Red);
}

#[test]
fn landed_and_discarded_sessions_leave_the_sidebar() {
    let mut tui = Harness::new();
    tui.sessions(vec![
        session("webshop", "still-here"),
        in_phase(session("webshop", "gone-landed"), PhaseView::Landed),
        in_phase(session("webshop", "gone-discarded"), PhaseView::Discarded),
    ]);

    let screen = tui.screen();
    assert!(screen.contains("still-here"));
    assert!(!screen.contains("gone-landed"));
    assert!(!screen.contains("gone-discarded"));
}

#[test]
fn flags_are_marked_on_the_row() {
    let mut unseen = session("webshop", "unseen-one");
    unseen.flags.unseen = true;
    let mut flagged = with_agent(session("webshop", "flagged"), AgentStateView::Working);
    flagged.flags.stalled = true;
    flagged.flags.needs_rebase = true;
    flagged.flags.muted = true;
    let mut tui = Harness::new();
    tui.sessions(vec![unseen, flagged, session("webshop", "plain")]);

    assert!(tui.sidebar_line_with("unseen-one").contains('●'));
    assert!(!tui.sidebar_line_with("plain").contains('●'));
    let flags = tui.sidebar_lines().join("\n");
    let after = &flags[flags.find("flagged").unwrap()..];
    let row_block: String = after.lines().take(2).collect::<Vec<_>>().join(" ");
    assert!(row_block.contains("stalled?"), "{row_block}");
    assert!(row_block.contains("rebase"), "{row_block}");
    assert!(row_block.contains("muted"), "{row_block}");
}

#[test]
fn a_pr_session_shows_checks_review_and_new_comments() {
    let mut view = in_phase(session("webshop", "in-review"), PhaseView::PrOpen);
    view.agent = Some(AgentStateView::Idle);
    view.flags.pr_number = Some(57);
    view.flags.pr = Some(PrView {
        checks: PrChecksView::Failing,
        review: PrReviewView::ChangesRequested,
        new_comments: 2,
        closed: false,
    });
    let mut tui = Harness::new();
    tui.sessions(vec![view]);

    let screen = tui.screen();
    assert!(screen.contains("#57"), "{screen}");
    assert!(screen.contains("✗ checks"), "{screen}");
    assert!(screen.contains("changes requested"), "{screen}");
    assert!(screen.contains("2 new"), "{screen}");
}

#[test]
fn the_context_gauge_is_highlighted_near_auto_compaction() {
    let mut calm = session("webshop", "calm");
    calm.context_used_percent = Some(42.0);
    let mut full = session("webshop", "full");
    full.context_used_percent = Some(91.4);
    let mut tui = Harness::new();
    tui.sessions(vec![calm, full]);

    let screen = tui.screen();
    assert!(screen.contains("42%"), "{screen}");
    assert!(screen.contains("91%"), "{screen}");
    assert_ne!(tui.colour_of("42%"), Color::Red);
    assert_eq!(tui.colour_of("91%"), Color::Red);
}

#[test]
fn running_subagents_are_nested_under_their_session_and_finished_ones_collapse() {
    let subagent = |id: &str, kind: &str, description: &str, tools: u32, done: bool| SubagentView {
        id: id.into(),
        agent_type: kind.into(),
        description: description.into(),
        tool_count: tools,
        done,
    };
    let mut parent = with_agent(session("webshop", "parent"), AgentStateView::Working);
    parent.subagents = vec![
        subagent("a", "Explore", "find callers", 7, false),
        subagent("b", "Plan", "outline refactor", 3, true),
        subagent("c", "Explore", "read tests", 2, true),
    ];
    let mut tui = Harness::new();
    tui.sessions(vec![parent, session("webshop", "sibling")]);

    let lines = tui.sidebar_lines();
    let row = |needle: &str| lines.iter().position(|line| line.contains(needle)).unwrap();
    assert!(row("parent") < row("find callers"));
    assert!(row("find callers") < row("sibling"));
    let running = &lines[row("find callers")];
    assert!(running.contains("Explore"), "{running}");
    assert!(running.contains("7 tools"), "{running}");
    assert!(!lines.iter().any(|line| line.contains("outline refactor")));
    assert!(lines[row("2 done")].contains("↳"));
}
