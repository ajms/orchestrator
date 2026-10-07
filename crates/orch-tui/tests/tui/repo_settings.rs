use std::path::PathBuf;

use crossterm::event::KeyCode;
use orch_protocol::{AgentChoice, DefaultPreset, RepoSettings, Request};
use orch_tui::{Effect, Event, ReviewData, ReviewPurpose, TuiConfig};

use crate::common::*;
use crate::new_session::field;

fn settings(repo: &str, base: &str) -> RepoSettings {
    RepoSettings {
        repo: PathBuf::from(repo),
        agents: vec![agent_offering(
            "claude",
            &["careful", "plan"],
            Some("careful"),
        )],
        default_agent: "claude".into(),
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
            agent_offering("claude", &["careful", "plan"], Some("careful")),
            AgentChoice {
                unavailable: Some("unknown Agent \"antigravity\"".into()),
                ..agent_offering("antigravity", &["plan"], Some("careful"))
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

fn agy_settings(repo: &str) -> RepoSettings {
    let mut antigravity = agent_offering("antigravity", &["tight", "plan", "edits"], None);
    antigravity.presets[0].lacks_rules = true;
    antigravity.default_preset = Some(DefaultPreset {
        name: "edits".into(),
        unsupported: Some("auto".into()),
    });
    RepoSettings {
        agents: vec![
            agent_offering("claude", &["tight", "plan", "edits", "auto"], Some("auto")),
            antigravity,
        ],
        default_agent: "antigravity".into(),
        ..settings(repo, "main")
    }
}

fn to_preset_field(tui: &mut Harness) {
    tui.press(KeyCode::BackTab);
}

#[test]
fn the_preset_field_marks_a_preset_without_rules_for_the_chosen_agent() {
    let mut tui = Harness::with_config(config());
    tui.daemon().settings = vec![agy_settings("/home/me/recent")];
    tui.command("new");
    tui.keys("Fix it");
    to_preset_field(&mut tui);

    tui.press(KeyCode::Right);
    assert!(field(&mut tui, "Preset").contains("◂ tight (no antigravity rules) ▸"));
    tui.press(KeyCode::Right);
    assert!(field(&mut tui, "Preset").contains("◂ plan ▸"));
}

#[test]
fn the_preset_list_follows_the_chosen_agent_and_says_when_the_default_falls_back() {
    let mut tui = Harness::with_config(config());
    tui.daemon().settings = vec![agy_settings("/home/me/recent")];
    tui.command("new");
    tui.keys("Fix it");
    let preset = field(&mut tui, "Preset");
    assert!(
        preset.contains("◂ edits (default auto unsupported) ▸"),
        "{preset}"
    );

    to_preset_field(&mut tui);
    let mut cycled = Vec::new();
    for _ in 0..4 {
        tui.press(KeyCode::Right);
        cycled.push(field(&mut tui, "Preset"));
    }
    assert!(
        !cycled.iter().any(|shown| shown.contains("auto ▸")),
        "{cycled:?}"
    );

    tui.press(KeyCode::BackTab);
    tui.press(KeyCode::Right);
    assert!(field(&mut tui, "Agent").contains("◂ claude ▸"));
    let preset = field(&mut tui, "Preset");
    assert!(preset.contains("◂ auto (Repo default) ▸"), "{preset}");
    assert!(!preset.contains("no claude rules"), "{preset}");
}

#[test]
fn no_preset_is_offered_before_the_repos_settings_say_which_the_agent_can_run() {
    let mut tui = Harness::with_config(config());
    tui.command("new");
    tui.keys("Fix it");
    to_preset_field(&mut tui);

    tui.press(KeyCode::Right);
    assert!(field(&mut tui, "Preset").contains("◂ (Repo default) ▸"));
    tui.ctrl('s');
    let create = create_requests(&mut tui).pop().expect("no Session created");
    assert_eq!(create.preset, None);
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
