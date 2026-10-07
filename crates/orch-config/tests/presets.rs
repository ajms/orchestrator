mod common;

use common::{Fixture, claude};
use orch_config::{ConfigError, ConfigProblem, PresetError};
use orch_core::PermissionMode;

#[test]
fn user_presets_from_every_layer_join_the_built_ins() {
    let fx = Fixture::new();
    fx.global("[defaults.presets.careful]\nmode = \"default\"\ndeny = [\"WebFetch\"]\n");
    fx.repo_file("[presets.tight]\nmode = \"plan\"\ndeny = [\"Bash(rm *)\"]\n");

    let config = fx.untrusted();
    let careful = config.select_preset(Some("careful"), claude()).unwrap();
    assert_eq!(careful.mode, Some(PermissionMode::Default));
    assert_eq!(careful.rules["claude"].deny, ["WebFetch"]);
    let tight = config.select_preset(Some("tight"), claude()).unwrap();
    assert_eq!(tight.mode, Some(PermissionMode::Plan));
    assert_eq!(
        config.select_preset(Some("edits"), claude()).unwrap().mode,
        Some(PermissionMode::AcceptEdits)
    );
}

#[test]
fn preset_without_mode_inherits_the_users_claude_mode() {
    let fx = Fixture::new();
    fx.repo_file("[presets.guarded]\ndeny = [\"WebFetch\"]\n");
    let preset = fx
        .untrusted()
        .select_preset(Some("guarded"), claude())
        .unwrap();
    assert_eq!(preset.mode, None);
}

#[test]
fn personal_preset_replaces_the_repo_files_preset_of_the_same_name() {
    let fx = Fixture::new();
    fx.global(&format!(
        "[repos.{}.presets.tight]\nmode = \"default\"\n",
        fx.repo_key()
    ));
    fx.repo_file("[presets.tight]\nmode = \"plan\"\n");
    let preset = fx
        .untrusted()
        .select_preset(Some("tight"), claude())
        .unwrap();
    assert_eq!(preset.mode, Some(PermissionMode::Default));
}

#[test]
fn selection_prefers_the_new_session_choice_then_the_configured_default_then_inherit() {
    let fx = Fixture::new();
    assert_eq!(
        fx.untrusted().select_preset(None, claude()).unwrap().name,
        "inherit"
    );

    fx.global("[defaults]\npreset = \"ask\"\n");
    assert_eq!(
        fx.untrusted().select_preset(None, claude()).unwrap().name,
        "ask"
    );

    fx.repo_file("preset = \"plan\"\n");
    let config = fx.untrusted();
    assert_eq!(config.select_preset(None, claude()).unwrap().name, "plan");
    assert_eq!(
        config.select_preset(Some("edits"), claude()).unwrap().name,
        "edits"
    );
}

#[test]
fn selecting_an_unknown_preset_is_an_error() {
    let fx = Fixture::new();
    assert_eq!(
        fx.untrusted().select_preset(Some("yolo"), claude()),
        Err(PresetError::Unknown("yolo".into()))
    );
}

const REDUCED: &[PermissionMode] = &[
    PermissionMode::Default,
    PermissionMode::AcceptEdits,
    PermissionMode::Plan,
];

#[test]
fn a_default_preset_the_agent_cannot_express_falls_back_to_edits() {
    let fx = Fixture::new();
    fx.global("[defaults]\npreset = \"auto\"\n");
    let config = fx.untrusted();
    assert_eq!(config.select_preset(None, REDUCED).unwrap().name, "edits");
    assert_eq!(config.select_preset(None, claude()).unwrap().name, "auto");

    fx.global("[defaults]\npreset = \"locked\"\n[defaults.presets.locked]\nmode = \"dontAsk\"\n");
    let config = fx.untrusted();
    assert_eq!(config.select_preset(None, REDUCED).unwrap().name, "edits");
}

#[test]
fn an_untrusted_committed_default_the_agent_cannot_express_falls_back_without_trust() {
    let fx = Fixture::new();
    fx.repo_file("preset = \"auto\"\n");
    let config = fx.untrusted();
    assert!(!config.is_trusted());
    assert_eq!(config.select_preset(None, REDUCED).unwrap().name, "edits");
    assert_eq!(
        config.select_preset(None, claude()),
        Err(PresetError::Untrusted("auto".into()))
    );
}

#[test]
fn the_fallback_is_inherit_when_the_agent_cannot_express_edits_either() {
    let fx = Fixture::new();
    fx.global("[defaults]\npreset = \"plan\"\n");
    assert_eq!(
        fx.untrusted().select_preset(None, &[]).unwrap().name,
        "inherit"
    );
}

#[test]
fn choosing_a_preset_the_agent_cannot_express_is_an_error() {
    let fx = Fixture::new();
    assert_eq!(
        fx.untrusted().select_preset(Some("auto"), REDUCED),
        Err(PresetError::Unsupported("auto".into()))
    );
    assert_eq!(
        fx.untrusted()
            .select_preset(Some("plan"), REDUCED)
            .unwrap()
            .name,
        "plan"
    );
}

#[test]
fn agent_tables_hold_that_agents_rules_and_top_level_rules_stay_claudes() {
    let fx = Fixture::new();
    fx.repo_file(
        "[presets.tight]\nmode = \"plan\"\ndeny = [\"WebFetch\"]\n[presets.tight.antigravity]\ndeny = [\"command(rm)\", \"mcp(docs/*)\"]\n",
    );
    let tight = fx
        .untrusted()
        .select_preset(Some("tight"), claude())
        .unwrap();
    assert_eq!(tight.rules["claude"].deny, ["WebFetch"]);
    let agy = &tight.rules["antigravity"];
    assert_eq!(agy.deny, ["command(rm)", "mcp(docs/*)"]);
    assert!(agy.allow.is_empty());
}

#[test]
fn an_unknown_key_in_an_agent_rule_table_makes_the_config_invalid() {
    let fx = Fixture::new();
    fx.repo_file("[presets.tight.antigravity]\nalow = [\"command(ls)\"]\n");
    assert!(matches!(
        fx.loader.repo(fx.repo_path(), None),
        Err(ConfigError::Parse { .. })
    ));
}

#[test]
fn a_rule_table_for_an_unknown_agent_makes_the_config_invalid() {
    let fx = Fixture::new();
    fx.repo_file("[presets.tight.antigravty]\ndeny = [\"command(rm)\"]\n");
    let err = fx.loader.repo(fx.repo_path(), None).unwrap_err();
    assert!(matches!(
        &err,
        ConfigError::Invalid {
            problem: ConfigProblem::UnknownRuleAgent { agent, .. },
            ..
        } if agent == "antigravty"
    ));
}

#[test]
fn a_claude_rule_table_is_refused_because_top_level_rules_are_claudes() {
    let fx = Fixture::new();
    fx.repo_file("[presets.tight.claude]\ndeny = [\"WebFetch\"]\n");
    let err = fx.loader.repo(fx.repo_path(), None).unwrap_err();
    assert!(matches!(
        err,
        ConfigError::Invalid {
            problem: ConfigProblem::ClaudeRuleTable { .. },
            ..
        }
    ));
    assert!(err.to_string().contains("[presets.tight]"), "{err}");
}

#[test]
fn invalid_personal_preset_is_reported_against_the_global_file() {
    let fx = Fixture::new();
    fx.global("[defaults.presets.odd]\nmode = \"reckless\"\n");
    let err = fx.loader.repo(fx.repo_path(), None).unwrap_err();
    assert!(err.to_string().contains("config.toml"), "{err}");
}

#[test]
fn invalid_mode_or_reserved_name_makes_the_config_invalid() {
    let fx = Fixture::new();
    fx.repo_file("[presets.odd]\nmode = \"reckless\"\n");
    assert!(matches!(
        fx.loader.repo(fx.repo_path(), None),
        Err(ConfigError::Invalid {
            problem: ConfigProblem::UnknownPermissionMode { .. },
            ..
        })
    ));

    fx.repo_file("[presets.inherit]\nmode = \"plan\"\n");
    assert!(matches!(
        fx.loader.repo(fx.repo_path(), None),
        Err(ConfigError::Invalid {
            problem: ConfigProblem::ReservedPresetName,
            ..
        })
    ));
}
