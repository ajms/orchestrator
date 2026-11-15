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

pub fn type_title(tui: &mut Harness, title: &str) {
    tui.press(KeyCode::BackTab);
    tui.keys(title);
    tui.press(KeyCode::Tab);
}

pub fn shown_title(tui: &mut Harness) -> String {
    let shown = field(tui, "Title");
    let (head, value) = shown.split_once('\n').unwrap();
    let column = head[..head.find("│ Title").unwrap()].chars().count() + 1;
    value
        .chars()
        .skip(column)
        .take_while(|c| *c != '│')
        .collect::<String>()
        .trim()
        .to_string()
}

#[test]
fn the_branch_is_prefilled_from_the_title_until_edited() {
    let mut tui = form(config());
    tui.keys("Fix the login timeout");
    assert!(!field(&mut tui, "Branch").contains("fix-the-login"));

    type_title(&mut tui, "Login timeout");
    assert!(field(&mut tui, "Branch").contains("orch/login-timeout"));

    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    for _ in "-timeout".chars() {
        tui.press(KeyCode::Backspace);
    }
    tui.keys("-bug");
    tui.press(KeyCode::BackTab);
    tui.press(KeyCode::BackTab);
    tui.press(KeyCode::BackTab);
    tui.keys(" fix");
    assert!(field(&mut tui, "Branch").contains("orch/login-bug"));
}

#[test]
fn without_a_typed_title_a_random_one_is_suggested_dimmed_and_names_the_branch() {
    let mut tui = form(config());
    tui.keys("Fix the login timeout");
    let title = shown_title(&mut tui);
    assert_eq!(title.split(' ').count(), 2, "{title}");
    assert_eq!(tui.colour_of(&title), ratatui::style::Color::DarkGray);
    let slug = title.to_ascii_lowercase().replace(' ', "-");
    assert!(field(&mut tui, "Branch").contains(&format!("orch/{slug}")));

    submit(&mut tui);
    assert_eq!(create_requests(&mut tui)[0].title, Some(title));
}

#[test]
fn typing_a_title_replaces_the_suggestion_and_clearing_it_brings_it_back() {
    let mut tui = form(config());
    let suggested = shown_title(&mut tui);
    tui.press(KeyCode::BackTab);
    tui.keys("Login");
    assert_eq!(shown_title(&mut tui), "Login");
    assert_ne!(tui.colour_of("Login"), ratatui::style::Color::DarkGray);

    tui.ctrl('u');
    assert_eq!(shown_title(&mut tui), suggested);
    assert!(field(&mut tui, "Branch").contains(&suggested.to_ascii_lowercase().replace(' ', "-")));
}

#[test]
fn submitting_creates_a_session_with_defaults_left_to_the_daemon() {
    let mut tui = form(config());
    tui.keys("First line\nsecond line");
    type_title(&mut tui, "Two lines");
    submit(&mut tui);

    assert_eq!(
        create_requests(&mut tui),
        vec![CreateSession {
            title: Some("Two lines".into()),
            ..CreateSession::new("/home/me/recent", "First line\nsecond line")
        }]
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
    type_title(&mut tui, "Stacked work");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::End);
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
            title: Some("Stacked work".into()),
            branch: Some("orch/stacked-work-v2".into()),
            base: Some("orch/other-work".into()),
            preset: Some("plan".into()),
            agent: None,
            parent: None,
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
fn an_empty_prompt_starts_a_session_without_one() {
    let mut tui = form(config());
    let title = shown_title(&mut tui);
    submit(&mut tui);
    let create = create_requests(&mut tui).pop().expect("no Session created");
    assert_eq!(create.prompt, "");
    assert_eq!(create.title, Some(title));
    assert_eq!(create.branch, None);
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
    type_title(&mut tui, "Trust me");
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
            Request::CreateSession(CreateSession {
                title: Some("Trust me".into()),
                ..CreateSession::new("/home/me/recent", "Needs trust")
            }),
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
