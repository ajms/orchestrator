use orch_agent::{
    AgentAdapter, Argv, Capabilities, ClaudeCode, LaunchSpec, Preset, PresetSelection, Presets,
    ReservedPresetName, Rules, UnknownPreset,
};
use orch_core::PermissionMode;

fn user_preset(name: &str, mode: Option<PermissionMode>, allow: &[&str], deny: &[&str]) -> Preset {
    let preset = Preset {
        name: name.into(),
        mode,
        ..Preset::default()
    };
    match allow.is_empty() && deny.is_empty() {
        true => preset,
        false => with_agent_rules(preset, ClaudeCode::NAME, allow, deny),
    }
}

fn with_agent_rules(preset: Preset, agent: &str, allow: &[&str], deny: &[&str]) -> Preset {
    let mut preset = preset;
    preset.rules.insert(
        agent.into(),
        Rules {
            allow: allow.iter().map(|rule| rule.to_string()).collect(),
            deny: deny.iter().map(|rule| rule.to_string()).collect(),
        },
    );
    preset
}

#[test]
fn built_in_presets_are_permission_modes_without_extra_rules() {
    let presets = Presets::default();
    for (name, mode) in [
        ("plan", Some(PermissionMode::Plan)),
        ("ask", Some(PermissionMode::Default)),
        ("edits", Some(PermissionMode::AcceptEdits)),
        ("auto", Some(PermissionMode::Auto)),
        ("inherit", None),
    ] {
        let preset = presets.get(name).expect(name);
        assert_eq!(preset.mode, mode, "{name}");
        assert!(preset.rules.is_empty(), "{name}");
    }
}

#[test]
fn user_defined_presets_are_found_by_name_and_may_replace_built_ins() {
    let locked = user_preset(
        "locked",
        Some(PermissionMode::DontAsk),
        &["Bash(cargo test *)"],
        &["WebFetch"],
    );
    let edits = user_preset(
        "edits",
        Some(PermissionMode::AcceptEdits),
        &[],
        &["Bash(rm *)"],
    );
    let presets = Presets::new(vec![locked.clone(), edits.clone()]).unwrap();
    assert_eq!(presets.get("locked"), Some(&locked));
    assert_eq!(presets.get("edits"), Some(&edits));
    assert_eq!(
        presets.get("plan").map(|p| p.mode),
        Some(Some(PermissionMode::Plan))
    );
    assert_eq!(presets.get("nope"), None);
}

#[test]
fn selection_prefers_new_then_repo_default_then_global_default_then_inherit() {
    let presets = Presets::default();
    let pick = |new, repo_default, global_default| {
        presets
            .select(PresetSelection {
                new,
                repo_default,
                global_default,
            })
            .unwrap()
            .name
    };
    assert_eq!(pick(Some("plan"), Some("edits"), Some("auto")), "plan");
    assert_eq!(pick(None, Some("edits"), Some("auto")), "edits");
    assert_eq!(pick(None, None, Some("auto")), "auto");
    assert_eq!(pick(None, None, None), "inherit");
}

#[test]
fn selecting_an_unknown_preset_is_an_error_not_a_silent_fallback() {
    let selection = PresetSelection {
        new: None,
        repo_default: Some("yolo"),
        global_default: Some("edits"),
    };
    assert_eq!(
        Presets::default().select(selection),
        Err(UnknownPreset("yolo".into()))
    );
}

#[test]
fn a_preset_loosens_when_it_allows_more_than_asking() {
    let presets = Presets::default();
    let loosens = |name: &str| presets.get(name).unwrap().loosens();
    assert!(!loosens("inherit"));
    assert!(!loosens("plan"));
    assert!(!loosens("ask"));
    assert!(loosens("edits"));
    assert!(loosens("auto"));

    let bypass = user_preset("yolo", Some(PermissionMode::BypassPermissions), &[], &[]);
    let allowing = user_preset("tests", None, &["Bash(cargo test *)"], &[]);
    let locked = user_preset("locked", Some(PermissionMode::DontAsk), &[], &["WebFetch"]);
    assert!(bypass.loosens());
    assert!(allowing.loosens());
    assert!(!locked.loosens());
}

#[test]
fn a_preset_loosens_when_any_agents_allow_rules_are_non_empty() {
    let plan = user_preset("tight", Some(PermissionMode::Plan), &[], &[]);
    let agy_allows = with_agent_rules(plan.clone(), "antigravity", &["command(git)"], &[]);
    let agy_denies = with_agent_rules(plan, "antigravity", &[], &["command(rm)"]);
    assert!(agy_allows.loosens());
    assert!(!agy_denies.loosens());
}

struct Reduced;

impl AgentAdapter for Reduced {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            modes: true,
            ..Capabilities::default()
        }
    }

    fn launch(&self, _spec: &LaunchSpec) -> Argv {
        Argv {
            program: "reduced".into(),
            args: Vec::new(),
        }
    }

    fn modes(&self) -> &'static [PermissionMode] {
        &[
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Plan,
        ]
    }
}

#[test]
fn an_agent_is_offered_only_the_presets_whose_mode_it_can_express() {
    let presets = Presets::new(vec![
        user_preset("yolo", Some(PermissionMode::BypassPermissions), &[], &[]),
        user_preset("locked", Some(PermissionMode::DontAsk), &[], &["WebFetch"]),
        user_preset("guarded", None, &[], &["WebFetch"]),
        user_preset("careful", Some(PermissionMode::Default), &[], &[]),
    ])
    .unwrap();
    let offered = |adapter: &dyn AgentAdapter| {
        presets
            .offered(adapter.modes())
            .map(|preset| preset.name.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        offered(&Reduced),
        ["guarded", "careful", "plan", "ask", "edits", "inherit"]
    );
    assert_eq!(
        offered(&ClaudeCode::default()),
        presets.names().collect::<Vec<_>>()
    );
}

#[test]
fn a_preset_with_rules_lacks_them_for_an_agent_it_has_none_for() {
    let claude_only = user_preset("tight", Some(PermissionMode::Plan), &[], &["WebFetch"]);
    let agy_only = with_agent_rules(
        user_preset("agy", Some(PermissionMode::Plan), &[], &[]),
        "antigravity",
        &[],
        &["command(rm)"],
    );
    assert!(claude_only.lacks_rules("antigravity"));
    assert!(!claude_only.lacks_rules("claude"));
    assert!(agy_only.lacks_rules("claude"));
    assert!(!agy_only.lacks_rules("antigravity"));
    let plan = Presets::default().get("plan").unwrap().clone();
    assert!(!plan.lacks_rules("antigravity"));
}

#[test]
fn inherit_is_reserved_and_cannot_be_redefined() {
    let impostor = user_preset("inherit", Some(PermissionMode::BypassPermissions), &[], &[]);
    assert_eq!(
        Presets::new(vec![impostor]),
        Err(ReservedPresetName("inherit".into()))
    );
}

#[test]
fn preset_names_list_user_presets_then_built_ins() {
    let presets = Presets::new(vec![
        user_preset("careful", Some(PermissionMode::Default), &[], &[]),
        user_preset("plan", Some(PermissionMode::Plan), &["Read"], &[]),
    ])
    .unwrap();
    assert_eq!(
        presets.names().collect::<Vec<_>>(),
        ["careful", "plan", "ask", "edits", "auto", "inherit"]
    );
    assert_eq!(
        Presets::default().names().collect::<Vec<_>>(),
        ["plan", "ask", "edits", "auto", "inherit"]
    );
}
