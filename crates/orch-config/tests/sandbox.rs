mod common;

use common::Fixture;
use orch_config::{PresetError, TrustItem};

#[test]
fn every_preset_runs_in_the_sandbox_unless_it_opts_out() {
    let fx = Fixture::new();
    fx.global(
        "[defaults.presets.mine]\nmode = \"plan\"\n\n[defaults.presets.open]\nsandbox = false\n",
    );
    fx.repo_file("[presets.committed]\nmode = \"plan\"\n");

    let config = fx.untrusted();
    for name in [
        "plan",
        "ask",
        "edits",
        "auto",
        "inherit",
        "mine",
        "committed",
    ] {
        assert!(
            config.select_preset(Some(name)).unwrap().sandboxed(),
            "{name}"
        );
    }
    assert!(config.select_preset(None).unwrap().sandboxed());
    assert!(!config.select_preset(Some("open")).unwrap().sandboxed());
}

#[test]
fn a_committed_preset_that_leaves_the_sandbox_needs_trust() {
    let fx = Fixture::new();
    fx.repo_file("[presets.open]\nmode = \"plan\"\nsandbox = false\n");

    let untrusted = fx.untrusted();
    assert_eq!(
        untrusted.select_preset(Some("open")),
        Err(PresetError::Untrusted("open".into()))
    );
    assert!(matches!(
        &untrusted.trust_request().unwrap().items[..],
        [TrustItem::Preset(preset)] if preset.name == "open"
    ));
    assert_eq!(
        fx.approved().select_preset(Some("open")).unwrap().name,
        "open"
    );
}

#[test]
fn a_committed_preset_that_stays_in_the_sandbox_needs_no_trust() {
    let fx = Fixture::new();
    fx.repo_file("[presets.boxed]\nmode = \"plan\"\nsandbox = true\n");
    assert_eq!(fx.untrusted().trust_request(), None);
}

#[test]
fn leaving_the_sandbox_in_personal_or_global_config_needs_no_trust() {
    let fx = Fixture::new();
    fx.global(&format!(
        "[defaults.presets.open]\nsandbox = false\n\n[repos.{}.presets.mine]\nsandbox = false\n",
        fx.repo_key()
    ));
    let config = fx.untrusted();
    assert_eq!(config.trust_request(), None);
    assert!(config.select_preset(Some("mine")).is_ok());
}

#[test]
fn approval_lapses_when_a_committed_preset_starts_leaving_the_sandbox() {
    let fx = Fixture::new();
    fx.repo_file("[presets.yolo]\nmode = \"auto\"\n");
    let hash = fx.untrusted().trust_request().unwrap().hash;

    fx.repo_file("[presets.yolo]\nmode = \"auto\"\nsandbox = false\n");
    let config = fx.loader.repo(fx.repo_path(), Some(&hash)).unwrap();
    assert!(!config.is_trusted());
}

#[test]
fn the_global_switch_turns_off_the_sandbox_for_presets_that_do_not_say() {
    let fx = Fixture::new();
    fx.global("[defaults]\nsandbox = false\n\n[defaults.presets.boxed]\nsandbox = true\n");
    let config = fx.untrusted();
    assert!(config.select_preset(Some("boxed")).unwrap().sandboxed());
    assert!(!config.select_preset(Some("plan")).unwrap().sandboxed());
    assert!(!config.select_preset(None).unwrap().sandboxed());
}

#[test]
fn the_global_switch_cannot_be_committed() {
    let fx = Fixture::new();
    fx.repo_file("sandbox = false\n");
    let err = fx.loader.repo(fx.repo_path(), None).unwrap_err();
    assert!(
        err.to_string().contains("sandbox is not allowed here"),
        "{err}"
    );
}
