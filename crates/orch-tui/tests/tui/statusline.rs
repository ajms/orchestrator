use orch_protocol::AgentStateView;
use orch_tui::{Event, RateLimits};
use ratatui::style::Color;

use crate::common::*;

fn statusline(tui: &mut Harness) -> String {
    tui.lines().pop().unwrap()
}

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

#[test]
fn a_rate_limit_badge_appears_above_80_percent_and_turns_red_at_95() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "a")]);
    tui.send(Event::RateLimits(RateLimits {
        five_hour: Some(79.0),
        seven_day: None,
    }));
    assert!(!statusline(&mut tui).contains("5h"));

    tui.send(Event::RateLimits(RateLimits {
        five_hour: Some(83.2),
        seven_day: Some(50.0),
    }));
    let status = statusline(&mut tui);
    assert!(status.contains("5h 83%"), "{status}");
    assert!(!status.contains("7d"), "{status}");
    assert_ne!(tui.colour_of("5h 83%"), Color::Red);

    tui.send(Event::RateLimits(RateLimits {
        five_hour: Some(83.2),
        seven_day: Some(96.0),
    }));
    assert!(statusline(&mut tui).contains("7d 96%"));
    assert_eq!(tui.colour_of("7d 96%"), Color::Red);
}
