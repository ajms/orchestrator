use crossterm::event::{KeyCode, MouseEventKind};
use orch_protocol::{
    AgentStateView, FromDaemon, PhaseView, Request, SessionView, SubagentTranscript,
    TranscriptEntry,
};
use orch_tui::Event;
use ratatui::style::Color;

use crate::common::*;

fn parent(done: [bool; 2]) -> SessionView {
    let mut parent = with_agent(session("webshop", "parent"), AgentStateView::Working);
    parent.subagents = vec![
        subagent("a", "Explore", "find callers", done[0]),
        subagent("b", "Plan", "outline refactor", done[1]),
    ];
    parent
}

fn on_subagent() -> Harness {
    let mut tui = Harness::new();
    tui.sessions(vec![parent([false, false]), session("webshop", "below")]);
    tui.keys("J");
    tui
}

fn deliver(tui: &mut Harness, session: &str, subagent: &str, entries: Vec<TranscriptEntry>) {
    stream(tui, session, subagent, entries, false);
}

fn stream(
    tui: &mut Harness,
    session: &str,
    subagent: &str,
    entries: Vec<TranscriptEntry>,
    replace: bool,
) {
    tui.send(Event::Daemon(FromDaemon::SubagentTranscript(
        SubagentTranscript {
            session: id(session),
            subagent: subagent.into(),
            entries,
            replace,
        },
    )));
}

fn backlog(tui: &mut Harness, entries: Vec<TranscriptEntry>) {
    stream(tui, "parent", "a", entries, true);
}

fn text(text: &str) -> TranscriptEntry {
    TranscriptEntry::Text { text: text.into() }
}

fn call(id: &str, tool: &str, argument: &str) -> TranscriptEntry {
    TranscriptEntry::ToolCall {
        id: id.into(),
        tool: tool.into(),
        argument: Some(argument.into()),
    }
}

fn result(id: &str, text: &str, error: bool) -> TranscriptEntry {
    TranscriptEntry::ToolResult {
        id: id.into(),
        text: text.into(),
        error,
    }
}

fn numbered(prefix: &str, count: usize) -> String {
    (1..=count)
        .map(|n| format!("{prefix} {n:02}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn shows(tui: &mut Harness, needle: &str) -> bool {
    tui.pane_lines().iter().any(|line| line.contains(needle))
}

fn subscriptions(tui: &mut Harness) -> Vec<Request> {
    tui.daemon()
        .requests()
        .into_iter()
        .filter(|request| {
            matches!(
                request,
                Request::SubscribeSubagent { .. } | Request::UnsubscribeSubagent
            )
        })
        .collect()
}

fn subscribe(session: &str, subagent: &str) -> Request {
    Request::SubscribeSubagent {
        session: id(session),
        subagent: subagent.into(),
    }
}

fn long_transcript() -> Harness {
    let mut tui = on_subagent();
    backlog(&mut tui, vec![text(&numbered("row", 60))]);
    tui
}

#[test]
fn selecting_a_subagent_subscribes_to_its_transcript_and_leaving_unsubscribes() {
    let mut tui = on_subagent();
    assert_eq!(subscriptions(&mut tui), vec![subscribe("parent", "a")]);

    tui.keys("J");
    assert_eq!(
        subscriptions(&mut tui),
        vec![subscribe("parent", "a"), subscribe("parent", "b")]
    );

    tui.keys("K");
    tui.keys("K");
    assert_eq!(
        subscriptions(&mut tui),
        vec![
            subscribe("parent", "a"),
            subscribe("parent", "b"),
            subscribe("parent", "a"),
            Request::UnsubscribeSubagent,
        ]
    );
}

#[test]
fn the_header_and_title_name_the_subagent_its_status_and_its_parent() {
    let mut tui = on_subagent();

    assert!(shows(&mut tui, "Explore: find callers · running · 3 tools"));
    let title = tui.pane_lines()[0].clone();
    assert!(title.contains("parent"), "{title}");
    assert!(title.contains("Explore"), "{title}");
    assert!(!title.contains("running"), "{title}");

    tui.changed(parent([true, false]));
    assert!(shows(&mut tui, "Explore: find callers · done · 3 tools"));
}

#[test]
fn renders_the_prompt_text_and_tool_calls_with_their_results() {
    let mut tui = on_subagent();

    backlog(
        &mut tui,
        vec![
            TranscriptEntry::Prompt {
                text: "find every caller of render".into(),
            },
            text("Looking at the render module."),
            call("t1", "Read", "src/render.rs"),
            call("t2", "Grep", "fn render"),
            result("t2", "src/app.rs:12", false),
            result("t1", "fn render() {}", false),
        ],
    );

    assert!(shows(&mut tui, "find every caller of render"));
    assert!(shows(&mut tui, "Looking at the render module."));
    assert!(shows(&mut tui, "● Read(src/render.rs)"));
    let lines = tui.pane_lines();
    let at = |needle: &str| lines.iter().position(|line| line.contains(needle)).unwrap();
    assert_eq!(at("fn render() {}"), at("src/render.rs") + 1);
    assert_eq!(at("src/app.rs:12"), at("Grep") + 1);
}

#[test]
fn tool_errors_are_marked() {
    let mut tui = on_subagent();

    backlog(
        &mut tui,
        vec![
            call("t1", "Bash", "rm -rf target"),
            result("t1", "permission denied", true),
        ],
    );

    assert!(tui.line_with("permission denied").contains("error"));
    assert_eq!(tui.colour_of("permission denied"), Color::Red);
}

#[test]
fn a_call_shows_its_tool_with_the_argument_in_parentheses() {
    let mut tui = on_subagent();

    backlog(&mut tui, vec![call("t1", "Bash", "cargo test")]);

    assert!(shows(&mut tui, "● Bash(cargo test)"));
}

#[test]
fn a_multi_line_argument_shows_its_first_line() {
    let mut tui = on_subagent();

    backlog(
        &mut tui,
        vec![call("t1", "Bash", "cat <<END\nprint(1)\nEND")],
    );

    assert!(shows(&mut tui, "● Bash(cat <<END …)"));
    assert!(!shows(&mut tui, "print(1)"));
}

#[test]
fn the_bullet_shows_whether_a_call_is_pending_done_or_failed() {
    let mut tui = on_subagent();

    backlog(
        &mut tui,
        vec![
            call("t1", "Read", "done.rs"),
            result("t1", "fn done() {}", false),
            call("t2", "Bash", "failed"),
            result("t2", "boom", true),
            call("t3", "Grep", "pending"),
        ],
    );

    assert_eq!(tui.colour_of("● Read"), Color::Green);
    assert_eq!(tui.colour_of("● Bash"), Color::Red);
    assert_eq!(tui.colour_of("● Grep"), Color::DarkGray);
}

#[test]
fn multi_line_results_collapse_to_their_line_count() {
    let mut tui = on_subagent();

    backlog(
        &mut tui,
        vec![
            call("t1", "Bash", "cargo test"),
            result("t1", &numbered("out", 8), false),
        ],
    );

    assert!(shows(&mut tui, "⎿ 8 lines"));
    assert!(!shows(&mut tui, "out 01"));
}

#[test]
fn bold_and_code_in_text_are_styled_without_their_markers() {
    let mut tui = on_subagent();

    backlog(&mut tui, vec![text("a **loud** word and `cargo fmt` here")]);

    assert!(shows(&mut tui, "a loud word and cargo fmt here"));
    assert_eq!(tui.colour_of("cargo fmt"), Color::Magenta);
}

#[test]
fn o_toggles_full_results_until_the_subagent_is_left() {
    let mut tui = on_subagent();
    let entries = vec![
        call("t1", "Bash", "cargo test"),
        result("t1", &numbered("out", 8), false),
    ];
    backlog(&mut tui, entries.clone());

    tui.keys("o");
    assert!(shows(&mut tui, "out 01"));
    assert!(shows(&mut tui, "out 08"));
    assert!(!shows(&mut tui, "8 lines"));

    tui.keys("o");
    assert!(!shows(&mut tui, "out 08"));

    tui.keys("o");
    tui.keys("KJ");
    backlog(&mut tui, entries);
    assert!(!shows(&mut tui, "out 08"));
    assert!(shows(&mut tui, "⎿ 8 lines"));
}

#[test]
fn live_appends_are_shown_at_the_tail() {
    let mut tui = long_transcript();
    assert!(shows(&mut tui, "row 60"));

    deliver(&mut tui, "parent", "a", vec![text("newest words")]);
    assert!(shows(&mut tui, "newest words"));

    backlog(&mut tui, vec![text("started over")]);
    assert!(shows(&mut tui, "started over"));
    assert!(!shows(&mut tui, "newest words"));
}

#[test]
fn messages_for_another_subagent_are_dropped() {
    let mut tui = on_subagent();
    backlog(&mut tui, vec![text("from a")]);

    deliver(&mut tui, "parent", "b", vec![text("from b")]);
    deliver(&mut tui, "below", "a", vec![text("from below")]);
    assert!(shows(&mut tui, "from a"));
    assert!(!shows(&mut tui, "from b"));
    assert!(!shows(&mut tui, "from below"));

    tui.keys("K");
    deliver(&mut tui, "parent", "a", vec![text("late")]);
    tui.keys("J");
    assert!(!shows(&mut tui, "from a"));
    assert!(!shows(&mut tui, "late"));
}

#[test]
fn enter_focuses_the_transcript_and_j_k_scroll_it() {
    let mut tui = long_transcript();

    tui.press(KeyCode::Enter);
    tui.keys("k");
    assert!(!shows(&mut tui, "row 60"));
    assert!(shows(&mut tui, "row 59"));
    assert!(highlighted(&mut tui, "find callers"));

    tui.keys("j");
    assert!(shows(&mut tui, "row 60"));
    assert!(highlighted(&mut tui, "find callers"));
}

#[test]
fn l_focuses_the_transcript_and_g_jumps_to_either_end() {
    let mut tui = long_transcript();

    tui.keys("lgg");
    assert!(shows(&mut tui, "row 01"));
    assert!(!shows(&mut tui, "row 60"));

    tui.keys("G");
    assert!(shows(&mut tui, "row 60"));
    assert!(!shows(&mut tui, "row 01"));
}

#[test]
fn ctrl_d_and_ctrl_u_scroll_half_a_page() {
    let mut tui = long_transcript();
    tui.keys("lgg");

    tui.ctrl('d');
    assert!(!shows(&mut tui, "row 12"));
    assert!(shows(&mut tui, "row 13"));

    tui.ctrl('u');
    assert!(shows(&mut tui, "row 01"));
}

#[test]
fn scrolling_up_stops_following_the_tail_until_g() {
    let mut tui = long_transcript();
    tui.keys("lk");

    deliver(&mut tui, "parent", "a", vec![text("newest words")]);
    assert!(!shows(&mut tui, "newest words"));
    assert!(shows(&mut tui, "row 59"));

    tui.keys("G");
    assert!(shows(&mut tui, "newest words"));
}

#[test]
fn the_wheel_scrolls_the_transcript() {
    let mut tui = long_transcript();
    let (col, row) = in_pane(5, 5);

    tui.wheel(MouseEventKind::ScrollUp, col, row);
    assert!(!shows(&mut tui, "row 60"));

    tui.wheel(MouseEventKind::ScrollDown, col, row);
    assert!(shows(&mut tui, "row 60"));
}

#[test]
fn esc_and_h_go_back_to_the_parent_session_in_the_sidebar() {
    for key in [KeyCode::Esc, KeyCode::Char('h')] {
        let mut tui = long_transcript();
        tui.press(KeyCode::Enter);

        tui.press(key);
        assert!(highlighted(&mut tui, "parent"));
        assert_eq!(tui.colour_at(0, 0), Color::Cyan, "the sidebar is focused");
        assert!(!shows(&mut tui, "row 60"));

        tui.keys("j");
        assert_eq!(shown(&mut tui).as_deref(), Some("below"));
    }
}

#[test]
fn i_on_a_subagent_of_a_session_without_a_live_agent_stays_in_normal_mode() {
    let mut failed = in_phase(session("webshop", "parent"), PhaseView::SetupFailed);
    failed.subagents = vec![subagent("a", "Explore", "find callers", false)];
    let mut tui = Harness::new();
    tui.sessions(vec![failed]);
    tui.keys("J");
    assert!(highlighted(&mut tui, "find callers"));

    tui.keys("i");

    assert!(statusline(&mut tui).contains("NORMAL"));
    assert!(highlighted(&mut tui, "parent"));
}

#[test]
fn v_on_a_subagent_does_not_select_in_the_hidden_agent_pane() {
    let mut tui = long_transcript();

    tui.keys("lv");

    assert!(!statusline(&mut tui).contains("VISUAL"));
}

fn highlighted(tui: &mut Harness, needle: &str) -> bool {
    tui.sidebar_background_of(needle) != Color::Reset
}

#[test]
fn the_statusline_lists_the_subagent_keys_for_the_focused_side() {
    let mut tui = on_subagent();

    let status = statusline(&mut tui);
    assert!(
        status.contains("Enter/l focus · J/K subagents · o full results · Esc/h back · i insert"),
        "{status}"
    );

    tui.press(KeyCode::Enter);
    let status = statusline(&mut tui);
    assert!(
        status.contains("j/k Ctrl-d/u gg/G scroll · o full results · Esc/h back · i insert"),
        "{status}"
    );
}

#[test]
fn tabs_are_expanded_and_control_sequences_dropped() {
    let mut tui = on_subagent();

    backlog(
        &mut tui,
        vec![
            text("name\tvalue"),
            call("t1", "Bash", "ls"),
            result("t1", "\x1b[31mred\x1b[0m plain\r\x07", false),
        ],
    );

    assert!(shows(&mut tui, "name    value"));
    assert!(shows(&mut tui, "⎿ red plain"));
    assert!(!shows(&mut tui, "[31m"));
}

#[test]
fn long_lines_wrap_inside_the_pane() {
    let mut tui = on_subagent();

    backlog(
        &mut tui,
        vec![text(&format!("{} wrapped tail", "x".repeat(100)))],
    );

    assert!(shows(&mut tui, "wrapped tail"));
}
