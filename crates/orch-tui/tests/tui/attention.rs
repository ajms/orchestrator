use orch_notify::terminal_attention;
use orch_protocol::FromDaemon;
use orch_tui::{Effect, Event};

use crate::common::*;

#[test]
fn a_ring_from_the_daemon_emits_osc_777_to_the_outer_terminal() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first")]);
    tui.send(Event::Daemon(FromDaemon::Ring {
        session: id("first"),
        title: "webshop / first".into(),
        body: "Needs input".into(),
    }));
    assert!(
        tui.take_effects()
            .contains(&Effect::WriteTerminal(terminal_attention(
                "webshop / first",
                "Needs input"
            )))
    );
}

#[test]
fn a_muted_session_does_not_ring() {
    let mut muted = session("webshop", "quiet");
    muted.flags.muted = true;
    let mut tui = Harness::new();
    tui.sessions(vec![muted]);
    tui.send(Event::Daemon(FromDaemon::Ring {
        session: id("quiet"),
        title: "webshop / quiet".into(),
        body: "Idle".into(),
    }));
    assert!(
        !tui.take_effects()
            .iter()
            .any(|effect| matches!(effect, Effect::WriteTerminal(_)))
    );
}

#[test]
fn a_version_mismatch_offers_to_restart_the_daemon() {
    let mut tui = Harness::new();
    tui.send(Event::VersionMismatch {
        message: "the running Daemon speaks protocol 1 but this Client speaks 2".into(),
    });
    let screen = tui.screen();
    assert!(screen.contains("speaks protocol 1"), "{screen}");
    assert!(screen.contains("r restart the Daemon"), "{screen}");

    tui.keys("r");
    assert!(tui.take_effects().contains(&Effect::RestartDaemon));
    tui.send(Event::VersionMismatch {
        message: "still old".into(),
    });
    tui.keys("q");
    assert!(tui.take_effects().contains(&Effect::Quit));
}

#[test]
fn a_long_version_mismatch_message_still_shows_the_keys() {
    let mut tui = Harness::new();
    tui.send(Event::VersionMismatch {
        message: "the running Daemon speaks protocol 7 but this Client speaks 8; restart the Daemon to upgrade it".repeat(2),
    });
    let screen = tui.screen();
    assert!(screen.contains("r restart the Daemon · q quit"), "{screen}");
}

#[test]
fn losing_the_daemon_is_shown() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first")]);
    tui.send(Event::Disconnected {
        reason: "connection reset".into(),
    });
    assert!(tui.screen().contains("lost the Daemon: connection reset"));
}

#[test]
fn a_notice_from_the_runtime_is_shown_in_the_statusline() {
    let mut tui = Harness::new();
    tui.send(Event::Notice("external review exited with status 1".into()));
    assert!(
        tui.lines()
            .pop()
            .unwrap()
            .contains("external review exited with status 1")
    );
}
