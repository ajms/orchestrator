use orch_protocol::AgentStateView;
use orch_protocol::FromDaemon;
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

#[test]
fn a_rate_limit_badge_appears_above_80_percent_and_turns_red_at_95() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "a")]);
    tui.send(Event::Daemon(FromDaemon::RateLimits {
        five_hour: Some(79.0),
        seven_day: None,
    }));
    assert!(!statusline(&mut tui).contains("5h"));

    tui.send(Event::Daemon(FromDaemon::RateLimits {
        five_hour: Some(83.2),
        seven_day: Some(50.0),
    }));
    let status = statusline(&mut tui);
    assert!(status.contains("5h 83%"), "{status}");
    assert!(!status.contains("7d"), "{status}");
    assert_ne!(tui.colour_of("5h 83%"), Color::Red);

    tui.send(Event::Daemon(FromDaemon::RateLimits {
        five_hour: Some(83.2),
        seven_day: Some(96.0),
    }));
    assert!(statusline(&mut tui).contains("7d 96%"));
    assert_eq!(tui.colour_of("7d 96%"), Color::Red);
}

fn noticed(session_id: &str) -> String {
    let mut tui = Harness::new();
    tui.sessions(vec![
        session("webshop", "first"),
        session("webshop", "second"),
    ]);
    tui.send(Event::Daemon(FromDaemon::Notice {
        session: id(session_id),
        text: "Broker: pushed".into(),
    }));
    statusline(&mut tui)
}

#[test]
fn a_notice_for_the_shown_session_appears_in_the_statusline() {
    let status = noticed("first");
    assert!(status.contains("Broker: pushed"), "{status}");
    assert!(!status.contains("first"), "{status}");
}

#[test]
fn a_notice_for_another_session_names_it() {
    let status = noticed("second");
    assert!(status.contains("second: Broker: pushed"), "{status}");
}
