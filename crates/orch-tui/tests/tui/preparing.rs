use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::KeyCode;
use orch_protocol::FromDaemon;
use orch_protocol::{Reply, RepoSettings, RequestError};
use orch_tui::{Event, TuiConfig};
use ratatui::style::Color;

use crate::common::*;

fn submitted(prompt: &str) -> Harness {
    replying(prompt, Ok(Reply::Done))
}

fn replying(prompt: &str, reply: Result<Reply, RequestError>) -> Harness {
    let mut tui = Harness::with_config(TuiConfig {
        repos: vec![PathBuf::from("/home/me/webshop")],
        ..TuiConfig::default()
    });
    tui.sessions(vec![session("webshop", "existing")]);
    tui.daemon().script_reply(reply);
    tui.daemon().hold = true;
    tui.command("new");
    tui.keys(prompt);
    tui.ctrl('s');
    tui
}

fn sidebar_row(tui: &mut Harness, needle: &str) -> usize {
    tui.sidebar_lines()
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not in the sidebar"))
}

#[test]
fn a_submitted_session_is_preparing_under_its_repo() {
    let mut tui = submitted("Fix login timeout");

    assert!(
        tui.sidebar_line_with("fix-login-timeout")
            .contains("Preparing… 0s")
    );
    assert!(sidebar_row(&mut tui, "fix-login-timeout") > sidebar_row(&mut tui, "webshop"));
}

#[test]
fn the_elapsed_time_counts_up_while_preparing() {
    let mut tui = submitted("Fix login timeout");
    assert_eq!(tui.tui.tick_every(), Some(Duration::from_secs(1)));

    tui.later(4_200);
    tui.tick();
    assert!(
        tui.sidebar_line_with("fix-login-timeout")
            .contains("Preparing… 4s")
    );
}

#[test]
fn the_cursor_moves_to_the_preparing_session_and_the_pane_describes_it() {
    let mut tui = submitted("Fix login timeout");

    assert_ne!(tui.sidebar_background_of("fix-login-timeout"), Color::Reset);
    assert_eq!(tui.sidebar_background_of("existing"), Color::Reset);
    let screen = tui.screen();
    assert!(screen.contains("webshop / fix-login-timeout"), "{screen}");
    assert!(screen.contains("Preparing the Worktree…"), "{screen}");
    assert!(screen.contains("Fix login timeout"), "{screen}");
}

fn submitted_and_created(prompt: &str) -> Harness {
    replying(
        prompt,
        Ok(Reply::Created {
            session: id("fix-login-timeout"),
        }),
    )
}

#[test]
fn the_row_gives_way_to_the_session_once_it_is_listed() {
    let mut tui = submitted_and_created("Fix login timeout");
    tui.release_replies();
    assert!(tui.screen().contains("Preparing"));

    tui.changed(session("webshop", "fix-login-timeout"));
    let screen = tui.screen();
    assert!(!screen.contains("Preparing"), "{screen}");
    assert_eq!(
        tui.daemon().last_view(),
        Some((Some("fix-login-timeout".into()), true))
    );
}

#[test]
fn a_session_listed_before_the_reply_takes_over_when_the_reply_arrives() {
    let mut tui = submitted_and_created("Fix login timeout");
    tui.changed(session("webshop", "fix-login-timeout"));
    tui.release_replies();

    let screen = tui.screen();
    assert!(!screen.contains("Preparing"), "{screen}");
    assert_eq!(
        tui.daemon().last_view(),
        Some((Some("fix-login-timeout".into()), true))
    );
}

fn refused(prompt: &str) -> Harness {
    let message = "branch orch/fix-login-timeout already exists".to_string();
    let mut tui = replying(prompt, Err(RequestError::Refused { message }));
    tui.release_replies();
    tui
}

#[test]
fn a_refused_creation_turns_the_row_into_an_error() {
    let mut tui = refused("Fix login timeout");

    let row = tui.sidebar_line_with("fix-login-timeout");
    assert!(row.contains("✗ branch"), "{row}");
    assert_eq!(tui.sidebar_colour_of("✗ branch"), Color::Red);
    let screen = tui.screen();
    assert!(screen.contains("Preparing failed"), "{screen}");
    assert!(
        screen.contains("branch orch/fix-login-timeout already exists"),
        "{screen}"
    );
    assert!(
        screen.contains("Enter reopens the form · x dismisses"),
        "{screen}"
    );
}

#[test]
fn enter_on_a_failed_row_reopens_the_form_filled_in() {
    let mut tui = Harness::with_config(TuiConfig {
        repos: vec![PathBuf::from("/home/me/webshop")],
        ..TuiConfig::default()
    });
    tui.sessions(vec![session("webshop", "existing")]);
    tui.daemon().settings = vec![RepoSettings {
        repo: PathBuf::from("/home/me/webshop"),
        agents: vec![agent_offering("claude", &["careful", "plan"], None)],
        default_agent: "claude".into(),
        default_base: None,
        review_command: None,
        branch_prefix: "orch/".into(),
        trust: None,
    }];
    let message = "branch orch/fix-login-timeout-v2 already exists".to_string();
    tui.daemon()
        .script_reply(Err(RequestError::Refused { message }));
    tui.command("new");
    tui.keys("Fix login timeout");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.keys("-v2");
    tui.press(KeyCode::Tab);
    tui.keys("release/1.2");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Right);
    tui.press(KeyCode::Right);
    tui.ctrl('s');

    tui.press(KeyCode::Enter);
    assert!(tui.screen().contains(":new Session"));
    assert!(!tui.screen().contains("✗ branch"));
    tui.ctrl('s');

    let creates = create_requests(&mut tui);
    assert_eq!(creates.len(), 2);
    assert_eq!(creates[0], creates[1]);
    assert_eq!(creates[0].preset.as_deref(), Some("plan"));
}

#[test]
fn x_dismisses_a_failed_row() {
    let mut tui = refused("Fix login timeout");
    tui.keys("x");

    let screen = tui.screen();
    assert!(!screen.contains("fix-login-timeout"), "{screen}");
    assert!(!screen.contains(":new Session"), "{screen}");
}

#[test]
fn a_preparing_session_cannot_be_discarded_yet() {
    let mut tui = submitted("Fix login timeout");
    tui.command("discard");

    assert!(statusline(&mut tui).contains("it can be discarded once it is listed"));
    assert!(!tui.screen().contains(":discard"));
}

fn untrusted(prompt: &str) -> Harness {
    let mut tui = replying(
        prompt,
        Err(RequestError::Untrusted {
            repo: "/home/me/webshop".into(),
            hash: "abc123".into(),
            items: vec!["Setup script: ./scripts/setup.sh".into()],
        }),
    );
    tui.release_replies();
    tui
}

#[test]
fn an_untrusted_repo_marks_the_row_as_needing_trust() {
    let mut tui = untrusted("Fix login timeout");

    assert!(
        tui.sidebar_line_with("fix-login-timeout")
            .contains("needs Trust")
    );
}

#[test]
fn declining_trust_removes_the_row() {
    let mut tui = untrusted("Fix login timeout");
    tui.keys("n");

    let screen = tui.screen();
    assert!(!screen.contains("fix-login-timeout"), "{screen}");
}

#[test]
fn approving_trust_prepares_the_same_row_again() {
    let mut tui = untrusted("Fix login timeout");
    tui.daemon().script_reply(Ok(Reply::Done));
    tui.daemon().script_reply(Ok(Reply::Created {
        session: id("fix-login-timeout"),
    }));
    tui.keys("y");

    let rows: Vec<String> = tui
        .sidebar_lines()
        .into_iter()
        .filter(|line| line.contains("fix-login-timeout"))
        .collect();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].contains("Preparing…"), "{rows:?}");
    assert_eq!(create_requests(&mut tui).len(), 2);
}

#[test]
fn a_reply_without_a_session_fails_the_row() {
    let mut tui = submitted("Fix login timeout");
    tui.release_replies();

    assert!(tui.sidebar_line_with("fix-login-timeout").contains("✗"));
    assert_eq!(tui.tui.tick_every(), None);
}

#[test]
fn a_failed_trust_approval_fails_the_row() {
    let mut tui = untrusted("Fix login timeout");
    tui.daemon().script_reply(Err(RequestError::Refused {
        message: "config changed since it was shown".into(),
    }));
    tui.keys("y");

    let row = tui.sidebar_line_with("fix-login-timeout");
    assert!(row.contains("✗ config"), "{row}");
}

#[test]
fn preparing_in_a_folded_repo_unfolds_it_to_show_the_row() {
    let mut tui = Harness::with_config(TuiConfig {
        repos: vec![PathBuf::from("/home/me/webshop")],
        ..TuiConfig::default()
    });
    tui.sessions(vec![
        session("recent", "aaa"),
        session("webshop", "existing"),
    ]);
    tui.keys("j");
    tui.keys("za");
    tui.daemon().hold = true;
    tui.command("new");
    tui.keys("Fix login timeout");
    tui.ctrl('s');

    assert_ne!(tui.sidebar_background_of("fix-login-timeout"), Color::Reset);
    assert_eq!(tui.sidebar_background_of("aaa"), Color::Reset);
}

#[test]
fn dismissing_a_row_moves_the_cursor_to_its_neighbour() {
    let mut tui = Harness::with_config(TuiConfig {
        repos: vec![PathBuf::from("/home/me/webshop")],
        ..TuiConfig::default()
    });
    tui.sessions(vec![
        session("recent", "aaa"),
        session("webshop", "existing"),
    ]);
    tui.daemon().script_reply(Err(RequestError::Refused {
        message: "branch exists".into(),
    }));
    tui.command("new");
    tui.keys("Fix login timeout");
    tui.ctrl('s');
    tui.keys("x");

    assert_eq!(
        tui.daemon().last_view(),
        Some((Some("existing".into()), true))
    );
}

#[test]
fn a_trust_prompt_closed_another_way_fails_the_row() {
    let mut tui = untrusted("Fix login timeout");
    tui.send(Event::Daemon(FromDaemon::Focus {
        session: id("existing"),
    }));

    assert!(tui.sidebar_line_with("fix-login-timeout").contains("✗"));
}
