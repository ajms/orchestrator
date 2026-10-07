use std::path::PathBuf;

use crossterm::event::KeyCode;
use orch_protocol::{AgentChoice, RepoSettings, Request};
use orch_tui::{Effect, Event, ReviewData, ReviewPurpose, TuiConfig};

use crate::common::*;
use crate::new_session::field;

fn settings(repo: &str, base: &str) -> RepoSettings {
    RepoSettings {
        repo: PathBuf::from(repo),
        presets: vec!["careful".into(), "plan".into()],
        default_preset: Some("careful".into()),
        default_base: Some(base.into()),
        review_command: None,
        branch_prefix: "me/".into(),
        trust: None,
        ..RepoSettings::default()
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
    assert!(field(&mut tui, "Base").contains("develop"));
    assert!(field(&mut tui, "Preset").contains("careful"));
    assert!(field(&mut tui, "Branch").contains("me/fix-it"));
    tui.press(KeyCode::BackTab);
    tui.press(KeyCode::Right);
    assert!(field(&mut tui, "Preset").contains("◂ careful ▸"));
}

fn agent_settings(repo: &str) -> RepoSettings {
    RepoSettings {
        agents: vec![
            AgentChoice {
                name: "claude".into(),
                unavailable: None,
            },
            AgentChoice {
                name: "antigravity".into(),
                unavailable: Some("unknown Agent \"antigravity\"".into()),
            },
        ],
        default_agent: "antigravity".into(),
        ..settings(repo, "main")
    }
}

fn to_agent_field(tui: &mut Harness) {
    for _ in 0..4 {
        tui.press(KeyCode::Tab);
    }
}

#[test]
fn the_agent_field_starts_on_the_repos_default_agent_and_cycles_the_built_in_ones() {
    let mut tui = Harness::with_config(config());
    tui.daemon().settings = vec![agent_settings("/home/me/recent")];
    tui.command("new");
    tui.keys("Fix it");
    assert!(field(&mut tui, "Agent").contains("antigravity"));

    to_agent_field(&mut tui);
    tui.press(KeyCode::Right);
    assert!(field(&mut tui, "Agent").contains("◂ claude ▸"));
    tui.ctrl('s');

    let create = create_requests(&mut tui).pop().expect("no Session created");
    assert_eq!(create.agent.as_deref(), Some("claude"));
}

#[test]
fn an_unavailable_agent_is_refused_at_submit_with_the_reason() {
    let mut tui = Harness::with_config(config());
    tui.daemon().settings = vec![agent_settings("/home/me/recent")];
    tui.command("new");
    tui.keys("Fix it");
    tui.ctrl('s');

    assert!(create_requests(&mut tui).is_empty());
    let screen = tui.screen();
    assert!(screen.contains("unknown Agent \"antigravity\""), "{screen}");
}

#[test]
fn picking_another_repo_fetches_its_settings() {
    let mut tui = Harness::with_config(config());
    tui.daemon().settings = vec![
        settings("/home/me/recent", "develop"),
        settings("/home/me/older", "trunk"),
    ];
    tui.command("new");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Enter);
    tui.press(KeyCode::Down);
    tui.press(KeyCode::Enter);

    assert_eq!(
        settings_requested(&mut tui),
        [
            PathBuf::from("/home/me/recent"),
            PathBuf::from("/home/me/older")
        ]
    );
    assert!(field(&mut tui, "Base").contains("trunk"));
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
    assert!(field(&mut tui, "Repo").contains("/srv/first"));
    tui.press(KeyCode::Esc);

    tui.daemon().repos = Some(vec![PathBuf::from("/srv/after-restart")]);
    tui.sessions(Vec::new());
    tui.command("new");
    assert!(field(&mut tui, "Repo").contains("/srv/after-restart"));
}
