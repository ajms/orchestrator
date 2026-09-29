mod common;

use common::Fixture;
use orch_config::{ConfigError, ConfigProblem, PresetError};
use orch_core::PermissionMode;

#[test]
fn user_presets_from_every_layer_join_the_built_ins() {
    let fx = Fixture::new();
    fx.global("[defaults.presets.careful]\nmode = \"default\"\ndeny = [\"WebFetch\"]\n");
    fx.repo_file("[presets.tight]\nmode = \"plan\"\ndeny = [\"Bash(rm *)\"]\n");

    let config = fx.untrusted();
    let careful = config.select_preset(Some("careful")).unwrap();
    assert_eq!(careful.mode, Some(PermissionMode::Default));
    assert_eq!(careful.deny, vec!["WebFetch".to_string()]);
    let tight = config.select_preset(Some("tight")).unwrap();
    assert_eq!(tight.mode, Some(PermissionMode::Plan));
    assert_eq!(
        config.select_preset(Some("edits")).unwrap().mode,
        Some(PermissionMode::AcceptEdits)
    );
}

#[test]
fn preset_without_mode_inherits_the_users_claude_mode() {
    let fx = Fixture::new();
    fx.repo_file("[presets.guarded]\ndeny = [\"WebFetch\"]\n");
    let preset = fx.untrusted().select_preset(Some("guarded")).unwrap();
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
    let preset = fx.untrusted().select_preset(Some("tight")).unwrap();
    assert_eq!(preset.mode, Some(PermissionMode::Default));
}

#[test]
fn selection_prefers_the_new_session_choice_then_the_configured_default_then_inherit() {
    let fx = Fixture::new();
    assert_eq!(fx.untrusted().select_preset(None).unwrap().name, "inherit");

    fx.global("[defaults]\npreset = \"ask\"\n");
    assert_eq!(fx.untrusted().select_preset(None).unwrap().name, "ask");

    fx.repo_file("preset = \"plan\"\n");
    let config = fx.untrusted();
    assert_eq!(config.select_preset(None).unwrap().name, "plan");
    assert_eq!(config.select_preset(Some("edits")).unwrap().name, "edits");
}

#[test]
fn selecting_an_unknown_preset_is_an_error() {
    let fx = Fixture::new();
    assert_eq!(
        fx.untrusted().select_preset(Some("yolo")),
        Err(PresetError::Unknown("yolo".into()))
    );
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
