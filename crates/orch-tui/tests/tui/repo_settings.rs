use std::path::PathBuf;

use crossterm::event::KeyCode;
use orch_protocol::{RepoSettings, Request};
use orch_tui::{Effect, Event, ReviewData, ReviewPurpose, TuiConfig};

use crate::common::*;

fn settings(repo: &str, base: &str) -> RepoSettings {
    RepoSettings {
        repo: PathBuf::from(repo),
        presets: vec!["careful".into(), "plan".into()],
        default_preset: Some("careful".into()),
        default_base: Some(base.into()),
        review_command: None,
        branch_prefix: "me/".into(),
        trust: None,
    }
}

fn config() -> TuiConfig {
    TuiConfig {
        repos: vec![
            PathBuf::from("/home/me/recent"),
            PathBuf::from("/home/me/older"),
        ],
        ..TuiConfig::default()
    }
}

fn field_line(tui: &mut Harness, label: &str) -> String {
    tui.lines()
        .into_iter()
        .find(|line| line.contains(&format!(" {label} ")))
        .unwrap_or_else(|| panic!("no {label} field"))
}

fn settings_requested(tui: &mut Harness) -> Vec<PathBuf> {
    tui.daemon()
        .requests()
        .into_iter()
        .filter_map(|request| match request {
            Request::RepoSettings { repo } => Some(repo),
            _ => None,
        })
        .collect()
}

#[test]
fn the_new_form_shows_the_repos_resolved_default_base_presets_and_branch_prefix() {
    let mut tui = Harness::with_config(config());
    tui.daemon().settings = vec![settings("/home/me/recent", "develop")];
    tui.command("new");
    tui.keys("Fix it");

    assert_eq!(
        settings_requested(&mut tui),
        [PathBuf::from("/home/me/recent")]
    );
    assert!(field_line(&mut tui, "Base").contains("develop"));
    assert!(field_line(&mut tui, "Preset").contains("careful"));
    assert!(field_line(&mut tui, "Branch").contains("me/fix-it"));
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Right);
    assert!(field_line(&mut tui, "Preset").contains("◂ careful ▸"));
}

#[test]
fn picking_another_repo_fetches_its_settings() {
    let mut tui = Harness::with_config(config());
    tui.daemon().settings = vec![
        settings("/home/me/recent", "develop"),
        settings("/home/me/older", "trunk"),
    ];
    tui.command("new");
    tui.press(KeyCode::BackTab);
    tui.press(KeyCode::Right);

    assert_eq!(
        settings_requested(&mut tui),
        [
            PathBuf::from("/home/me/recent"),
            PathBuf::from("/home/me/older")
        ]
    );
    assert!(field_line(&mut tui, "Base").contains("trunk"));
}

#[test]
fn the_external_review_uses_the_repos_review_command_read_at_use() {
    let mut tui = Harness::with_config(config());
    tui.sessions(vec![session("webshop", "first")]);
    tui.daemon().settings = vec![RepoSettings {
        review_command: Some("nvim -d".into()),
        ..settings("/home/me/webshop", "main")
    }];
    tui.keys("D");

    assert_eq!(
        settings_requested(&mut tui),
        [PathBuf::from("/home/me/webshop")]
    );
    tui.send(Event::Review {
        session: id("first"),
        purpose: ReviewPurpose::External,
        result: Ok(ReviewData {
            files: Vec::new(),
            merge_base: "b45e".into(),
            tree: "7ree".into(),
        }),
    });
    assert!(tui.take_effects().iter().any(|effect| matches!(
        effect,
        Effect::RunExternal { command, .. } if command == "nvim -d"
    )));
}

#[test]
fn the_repo_list_is_fetched_again_whenever_the_daemon_connects() {
    let mut tui = Harness::with_config(config());
    tui.daemon().repos = Some(vec![PathBuf::from("/srv/first")]);
    tui.sessions(Vec::new());
    tui.command("new");
    assert!(field_line(&mut tui, "Repo").contains("/srv/first"));
    tui.press(KeyCode::Esc);

    tui.daemon().repos = Some(vec![PathBuf::from("/srv/after-restart")]);
    tui.sessions(Vec::new());
    tui.command("new");
    assert!(field_line(&mut tui, "Repo").contains("/srv/after-restart"));
}
