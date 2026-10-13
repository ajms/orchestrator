use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use orch_protocol::{CreateSession, Reply, Request, RequestError};
use orch_tui::{Effect, Event, TuiConfig};

use crate::common::*;

pub fn config() -> TuiConfig {
    TuiConfig {
        repos: vec![
            PathBuf::from("/home/me/recent"),
            PathBuf::from("/home/me/older"),
        ],
        cwd_repo: None,
        ..TuiConfig::default()
    }
}

pub fn form(config: TuiConfig) -> Harness {
    let mut tui = Harness::with_config(config);
    tui.sessions(vec![session("recent", "existing")]);
    tui.command("new");
    tui
}

fn submit(tui: &mut Harness) {
    tui.ctrl('s');
}

pub fn field(tui: &mut Harness, label: &str) -> String {
    let lines = tui.lines();
    let at = lines
        .iter()
        .position(|line| line.contains(&format!("│ {label} ")))
        .unwrap_or_else(|| panic!("no {label} field in\n{}", lines.join("\n")));
    format!("{}\n{}", lines[at], lines[at + 1])
}

#[test]
fn new_starts_on_the_selected_sessions_repo() {
    let mut tui = Harness::with_config(config());
    tui.sessions(vec![session("webshop", "existing")]);
    tui.command("new");
    let screen = tui.screen();
    assert!(screen.contains(":new Session"), "{screen}");
    assert!(field(&mut tui, "Repo").contains("/home/me/webshop"));
}

#[test]
fn new_starts_on_the_repo_heading_under_the_cursor() {
    let mut tui = Harness::with_config(config());
    tui.sessions(vec![session("older", "old")]);
    tui.keys("za");
    tui.command("new");
    assert!(field(&mut tui, "Repo").contains("/home/me/older"));
}

#[test]
fn without_a_selection_new_starts_on_the_working_directorys_repo() {
    let mut tui = Harness::with_config(TuiConfig {
        cwd_repo: Some(PathBuf::from("/home/me/older")),
        ..config()
    });
    tui.command("new");
    assert!(field(&mut tui, "Repo").contains("/home/me/older"));
}

#[test]
fn without_a_selection_or_working_directory_new_starts_on_the_most_recent_repo() {
    let mut tui = Harness::with_config(config());
    tui.command("new");
    assert!(field(&mut tui, "Repo").contains("/home/me/recent"));
}

#[test]
fn the_branch_is_prefilled_from_the_prompt_until_edited() {
    let mut tui = form(config());
    tui.keys("Fix the login timeout");
    assert!(field(&mut tui, "Branch").contains("orch/fix-the-login-timeout"));

    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    for _ in "-timeout".chars() {
        tui.press(KeyCode::Backspace);
    }
    tui.keys("-bug");
    tui.press(KeyCode::BackTab);
    tui.press(KeyCode::BackTab);
    tui.keys(" quickly");
    assert!(field(&mut tui, "Branch").contains("orch/fix-the-login-bug"));
}

#[test]
fn submitting_creates_a_session_with_defaults_left_to_the_daemon() {
    let mut tui = form(config());
    tui.keys("First line\nsecond line");
    submit(&mut tui);

    assert_eq!(
        create_requests(&mut tui),
        vec![CreateSession::new(
            "/home/me/recent",
            "First line\nsecond line"
        )]
    );
    assert!(!tui.screen().contains(":new Session"));
}

#[test]
fn an_edited_branch_base_and_preset_are_sent() {
    let mut other = session("recent", "other-work");
    other.branch = "orch/other-work".into();
    let mut tui = Harness::with_config(config());
    tui.sessions(vec![other]);
    tui.command("new");
    tui.keys("Stacked work");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.keys("-v2");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Down);
    assert!(field(&mut tui, "Base").contains("orch/other-work"));
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Right);
    assert!(field(&mut tui, "Preset").contains("plan"));
    submit(&mut tui);

    assert_eq!(
        create_requests(&mut tui),
        vec![CreateSession {
            repo: "/home/me/recent".into(),
            prompt: "Stacked work".into(),
            branch: Some("orch/stacked-work-v2".into()),
            base: Some("orch/other-work".into()),
            preset: Some("plan".into()),
        }]
    );
}

#[test]
fn a_base_can_be_typed() {
    let mut tui = form(config());
    tui.keys("Hotfix");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.keys("release/1.2");
    submit(&mut tui);
    assert_eq!(
        create_requests(&mut tui)[0].base.as_deref(),
        Some("release/1.2")
    );
}

#[test]
fn ctrl_g_edits_the_prompt_in_the_editor() {
    let mut tui = form(config());
    tui.keys("draft");
    tui.ctrl('g');
    assert!(tui.take_effects().contains(&Effect::EditText {
        text: "draft".into()
    }));

    tui.send(Event::EditorClosed(Ok(
        "Rewrite the parser\nwith tests".into()
    )));
    assert!(field(&mut tui, "Branch").contains("orch/rewrite-the-parser-with-tests"));
    submit(&mut tui);
    assert_eq!(
        create_requests(&mut tui)[0].prompt,
        "Rewrite the parser\nwith tests"
    );
}

#[test]
fn esc_with_an_empty_prompt_closes_the_form_at_once() {
    let mut tui = form(config());
    tui.press(KeyCode::Esc);
    assert!(!tui.screen().contains(":new Session"));
    assert!(create_requests(&mut tui).is_empty());
}

#[test]
fn esc_with_a_prompt_asks_once_before_discarding() {
    let mut tui = form(config());
    tui.keys("never mind");
    tui.press(KeyCode::Esc);
    assert!(tui.line_with(":new Session").contains(":new Session"));
    assert!(tui.screen().contains("Esc again to discard"));

    tui.press(KeyCode::Esc);
    assert!(!tui.screen().contains(":new Session"));
    assert!(create_requests(&mut tui).is_empty());
}

#[test]
fn another_key_after_esc_keeps_the_form() {
    let mut tui = form(config());
    tui.keys("never mind");
    tui.press(KeyCode::Esc);
    tui.keys("!");
    assert!(!tui.screen().contains("Esc again to discard"));
    tui.press(KeyCode::Esc);
    assert!(tui.screen().contains(":new Session"));
}

#[test]
fn an_empty_prompt_is_refused() {
    let mut tui = form(config());
    submit(&mut tui);
    assert!(create_requests(&mut tui).is_empty());
    assert!(tui.screen().contains("the prompt is empty"));
}

#[test]
fn the_created_session_is_selected_once_it_appears() {
    let mut tui = form(config());
    tui.daemon().script_reply(Ok(Reply::Created {
        session: id("brand-new"),
    }));
    tui.keys("Brand new");
    submit(&mut tui);
    tui.changed(session("recent", "brand-new"));

    assert_eq!(
        tui.daemon().last_view(),
        Some((Some("brand-new".into()), true))
    );
}

#[test]
fn an_untrusted_repo_asks_for_trust_and_retries_after_approval() {
    let mut tui = form(config());
    tui.daemon().script_reply(Err(RequestError::Untrusted {
        repo: "/home/me/recent".into(),
        hash: "abc123".into(),
        items: vec!["Setup script: ./scripts/setup.sh".into()],
    }));
    tui.keys("Needs trust");
    submit(&mut tui);

    let screen = tui.screen();
    assert!(screen.contains("Trust"), "{screen}");
    assert!(
        screen.contains("Setup script: ./scripts/setup.sh"),
        "{screen}"
    );
    tui.keys("y");

    let requests = tui.daemon().requests();
    let tail = &requests[requests.len() - 2..];
    assert_eq!(
        tail,
        &[
            Request::ApproveTrust {
                repo: "/home/me/recent".into(),
                hash: "abc123".into()
            },
            Request::CreateSession(CreateSession::new("/home/me/recent", "Needs trust")),
        ]
    );
}

#[test]
fn declining_trust_creates_nothing() {
    let mut tui = form(config());
    tui.daemon().script_reply(Err(RequestError::Untrusted {
        repo: "/home/me/recent".into(),
        hash: "abc123".into(),
        items: vec!["Setup script".into()],
    }));
    tui.keys("Needs trust");
    submit(&mut tui);
    tui.keys("n");

    assert_eq!(create_requests(&mut tui).len(), 1);
    assert!(!tui.screen().contains("Setup script"));
    let _ = KeyEvent::new(KeyCode::Null, KeyModifiers::NONE);
}
