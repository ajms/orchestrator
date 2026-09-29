use crossterm::event::KeyCode;
use orch_protocol::{Request, RequestError};
use orch_tui::Effect;

use crate::common::*;

fn one_session() -> Harness {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first")]);
    tui
}

fn statusline(tui: &mut Harness) -> String {
    tui.lines().pop().unwrap()
}

fn last_request(tui: &mut Harness) -> Request {
    tui.daemon().requests().pop().unwrap()
}

#[test]
fn colon_opens_the_command_line_and_esc_cancels_it() {
    let mut tui = one_session();
    tui.keys(":lan");
    let status = statusline(&mut tui);
    assert!(status.contains("COMMAND-LINE"), "{status}");
    assert!(status.contains(":lan"), "{status}");

    tui.press(KeyCode::Backspace);
    assert!(statusline(&mut tui).contains(":la"));
    tui.press(KeyCode::Esc);
    assert!(statusline(&mut tui).contains("NORMAL"));
}

#[test]
fn q_quits() {
    let mut tui = one_session();
    tui.command("q");
    assert!(tui.take_effects().contains(&Effect::Quit));
}

#[test]
fn resume_retry_and_start_act_on_the_selected_session() {
    let mut tui = one_session();
    tui.command("resume");
    assert_eq!(
        last_request(&mut tui),
        Request::Resume {
            session: id("first")
        }
    );
    tui.command("retry");
    assert_eq!(
        last_request(&mut tui),
        Request::RetrySetup {
            session: id("first")
        }
    );
    tui.command("start");
    assert_eq!(
        last_request(&mut tui),
        Request::StartAnyway {
            session: id("first")
        }
    );
}

#[test]
fn guards_off_and_on_toggle_the_sessions_guards() {
    let mut tui = one_session();
    tui.command("guards off");
    assert_eq!(
        last_request(&mut tui),
        Request::SetGuards {
            session: id("first"),
            enabled: false
        }
    );
    tui.command("guards on");
    assert_eq!(
        last_request(&mut tui),
        Request::SetGuards {
            session: id("first"),
            enabled: true
        }
    );
    tui.command("guards maybe");
    assert!(statusline(&mut tui).contains(":guards off|on"));
}

#[test]
fn commands_the_daemon_cannot_serve_yet_say_so() {
    let mut tui = one_session();
    for command in ["usage", "reconcile"] {
        let before = tui.daemon().requests().len();
        tui.command(command);
        let name = command.split(' ').next().unwrap();
        let status = statusline(&mut tui);
        assert!(
            status.contains(&format!(":{name} is not available yet")),
            "{status}"
        );
        assert_eq!(tui.daemon().requests().len(), before);
    }
}

#[test]
fn an_unknown_command_is_reported() {
    let mut tui = one_session();
    tui.command("frobnicate");
    assert!(statusline(&mut tui).contains("unknown command: frobnicate"));
}

#[test]
fn a_refused_request_shows_the_daemons_message() {
    let mut tui = one_session();
    tui.daemon().script_reply(Err(RequestError::Refused {
        message: "the Session is not Suspended".into(),
    }));
    tui.command("resume");
    assert!(statusline(&mut tui).contains("the Session is not Suspended"));
}

#[test]
fn preset_changes_the_sessions_preset() {
    let mut tui = one_session();
    tui.command("preset edits");
    assert_eq!(
        last_request(&mut tui),
        Request::SetPreset {
            session: id("first"),
            preset: "edits".into()
        }
    );
    tui.command("preset");
    assert!(statusline(&mut tui).contains("usage: :preset <name>"));
}

#[test]
fn mute_toggles_the_sessions_mute() {
    let mut muted = session("webshop", "quiet");
    muted.flags.muted = true;
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "loud"), muted]);
    tui.command("mute");
    assert_eq!(
        last_request(&mut tui),
        Request::SetMuted {
            session: id("loud"),
            muted: true
        }
    );
    tui.keys("j");
    tui.command("mute");
    assert_eq!(
        last_request(&mut tui),
        Request::SetMuted {
            session: id("quiet"),
            muted: false
        }
    );
}
