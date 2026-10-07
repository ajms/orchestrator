use orch_protocol::AgentStateView;
use orch_protocol::{AgentUsageWindows, FromDaemon, UsageWindowView};
use orch_tui::Event;
use ratatui::style::Color;

use crate::common::*;

#[test]
fn the_statusline_summarises_agent_states_and_unseen_sessions() {
    let mut unseen = with_agent(session("webshop", "a"), AgentStateView::NeedsInput);
    unseen.flags.unseen = true;
    let mut also_unseen = with_agent(session("webshop", "e"), AgentStateView::Errored);
    also_unseen.flags.unseen = true;
    let mut tui = Harness::new();
    tui.sessions(vec![
        unseen,
        with_agent(session("webshop", "b"), AgentStateView::NeedsInput),
        with_agent(session("webshop", "c"), AgentStateView::Working),
        with_agent(session("webshop", "d"), AgentStateView::Idle),
        also_unseen,
    ]);

    let status = statusline(&mut tui);
    assert!(status.contains("2 Needs input"), "{status}");
    assert!(status.contains("1 Working"), "{status}");
    assert!(status.contains("1 Errored"), "{status}");
    assert!(status.contains("● 2"), "{status}");
}

fn window(label: &str, used_percent: f64) -> UsageWindowView {
    UsageWindowView {
        name: format!("{label}-window"),
        label: label.into(),
        used_percent,
        resets_at_unix: None,
    }
}

fn windows(groups: &[(&str, &[UsageWindowView])]) -> Event {
    Event::Daemon(FromDaemon::UsageWindows {
        agents: groups
            .iter()
            .map(|(agent, windows)| AgentUsageWindows {
                agent: (*agent).into(),
                windows: windows.to_vec(),
            })
            .collect(),
    })
}

#[test]
fn the_statusline_shows_each_agents_usage_windows_as_one_group() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "a")]);
    assert!(!statusline(&mut tui).contains("claude"));

    tui.send(windows(&[
        ("claude", &[window("5h", 42.0), window("7d", 18.0)]),
        (
            "antigravity",
            &[window("gemini-wk", 7.0), window("3p-wk", 0.0)],
        ),
    ]));

    let status = statusline(&mut tui);
    assert!(
        status.contains("claude 5h 42% 7d 18% │ antigravity gemini-wk 7% 3p-wk 0%"),
        "{status}"
    );
}

#[test]
fn a_usage_window_turns_yellow_above_80_percent_and_red_at_95() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "a")]);
    tui.send(windows(&[(
        "claude",
        &[window("5h", 79.0), window("7d", 83.2)],
    )]));
    assert_ne!(tui.colour_of("5h 79%"), Color::Yellow);
    assert_eq!(tui.colour_of("7d 83%"), Color::Yellow);

    tui.send(windows(&[(
        "claude",
        &[window("5h", 79.0), window("7d", 96.0)],
    )]));
    assert_eq!(tui.colour_of("7d 96%"), Color::Red);
}

#[test]
fn a_usage_window_whose_reset_time_has_passed_is_left_out() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "a")]);
    let reset_at = |resets_at_unix, window| UsageWindowView {
        resets_at_unix: Some(resets_at_unix),
        ..window
    };

    tui.send(windows(&[
        (
            "claude",
            &[
                reset_at(1_000_000_000, window("5h", 97.0)),
                reset_at(32_000_000_000, window("7d", 18.0)),
            ],
        ),
        (
            "antigravity",
            &[reset_at(1_000_000_000, window("gemini-wk", 7.0))],
        ),
    ]));

    let status = statusline(&mut tui);
    assert!(status.contains("claude 7d 18%"), "{status}");
    assert!(!status.contains("5h"), "{status}");
    assert!(!status.contains("antigravity"), "{status}");
}
