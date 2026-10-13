mod common;

use common::Fixture;
use orch_config::{PresetError, SandboxList, TrustItem, Untrusted};

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

#[test]
fn sandbox_lists_append_across_layers() {
    let fx = Fixture::new();
    fx.global(&format!(
        "[defaults.sandbox]\nreadable = [\"~/.m2\"]\nwritable = [\"~/.cache\"]\n\n\
         [repos.{}.sandbox]\nwritable = [\"/srv/scratch\"]\nhidden = [\"/mnt/data\"]\n",
        fx.repo_key()
    ));
    fx.repo_file("[sandbox]\nreadable = [\"/opt/sdk\"]\nhidden = [\"~/.cargo\"]\n");

    let paths = fx.approved().sandbox_paths().unwrap();
    assert_eq!(paths.readable, [fx.home(".m2"), "/opt/sdk".into()]);
    assert_eq!(paths.writable, [fx.home(".cache"), "/srv/scratch".into()]);
    assert_eq!(paths.hidden, [fx.home(".cargo"), "/mnt/data".into()]);
}

#[test]
fn a_sandbox_path_must_be_absolute_or_start_with_home() {
    let fx = Fixture::new();
    fx.global("[defaults.sandbox]\nwritable = [\".cache\"]\n");
    let err = fx.loader.repo(fx.repo_path(), None).unwrap_err();
    assert!(err.to_string().contains("\".cache\""), "{err}");
}

#[test]
fn committed_readable_and_writable_paths_need_trust_before_a_sandboxed_spawn() {
    let fx = Fixture::new();
    fx.repo_file(
        "[sandbox]\nreadable = [\"~/.aws\"]\nwritable = [\"~/.bashrc\"]\nhidden = [\"~/.cargo\"]\n",
    );
    fx.global("[defaults.presets.open]\nsandbox = false\n");

    let untrusted = fx.untrusted();
    assert_eq!(
        untrusted.trust_request().unwrap().items,
        [
            TrustItem::SandboxPath(SandboxList::Readable, "~/.aws".into()),
            TrustItem::SandboxPath(SandboxList::Writable, "~/.bashrc".into()),
        ]
    );
    assert_eq!(untrusted.sandbox_paths(), Err(Untrusted));
    assert_eq!(
        untrusted.select_preset(Some("plan")),
        Err(PresetError::Untrusted("plan".into()))
    );
    assert!(untrusted.select_preset(Some("open")).is_ok());

    let approved = fx.approved();
    assert!(approved.select_preset(Some("plan")).unwrap().sandboxed());
    assert_eq!(
        approved.sandbox_paths().unwrap().readable,
        [fx.home(".aws")]
    );
}

#[test]
fn hidden_and_uncommitted_sandbox_paths_need_no_trust() {
    let fx = Fixture::new();
    fx.global(&format!(
        "[defaults.sandbox]\nwritable = [\"~/.cache\"]\n\n[repos.{}.sandbox]\nreadable = [\"~/.m2\"]\n",
        fx.repo_key()
    ));
    fx.repo_file("[sandbox]\nhidden = [\"~/.cargo\"]\n");
    let config = fx.untrusted();
    assert_eq!(config.trust_request(), None);
    assert!(config.select_preset(None).is_ok());
}

#[test]
fn approval_lapses_when_committed_readable_or_writable_paths_change_but_not_hidden_ones() {
    let fx = Fixture::new();
    fx.repo_file("[sandbox]\nreadable = [\"/opt/sdk\"]\n");
    let hash = fx.untrusted().trust_request().unwrap().hash;
    let trusted = |file: &str| {
        fx.repo_file(file);
        fx.loader
            .repo(fx.repo_path(), Some(&hash))
            .unwrap()
            .is_trusted()
    };

    assert!(trusted(
        "[sandbox]\nreadable = [\"/opt/sdk\"]\nhidden = [\"~/.cargo\"]\n"
    ));
    assert!(!trusted(
        "[sandbox]\nreadable = [\"/opt/sdk\", \"~/.aws\"]\n"
    ));
    assert!(!trusted("[sandbox]\nwritable = [\"/opt/sdk\"]\n"));
}

#[test]
fn naming_a_hard_wall_as_readable_or_writable_is_a_config_error() {
    let fx = Fixture::new();
    let home = fx.home("");
    let loader = orch_config::ConfigLoader::from_env_vars(|key| {
        (key == "HOME").then(|| home.display().to_string())
    })
    .unwrap();
    std::fs::create_dir_all(loader.global_path().parent().unwrap()).unwrap();
    let load = |list: &str, path: &str| {
        fx.repo_file(&format!("[sandbox]\n{list} = [{path:?}]\n"));
        loader.repo(fx.repo_path(), None)
    };

    for wall in [
        "/",
        "~/",
        "~/..",
        "~/.config",
        "~/.config/orchestrator",
        "~/.config/orchestrator/config.toml",
        "~/.local/state/orchestrator/sessions",
        "/srv/app/.git/hooks",
        "/srv/app/.git/hooks/pre-commit",
        "/srv/app/.git/config",
        "~/.claude/settings.json",
        "~/.claude/skills/mine",
    ] {
        for list in ["readable", "writable"] {
            let err = load(list, wall).unwrap_err();
            assert!(err.to_string().contains(&format!("{wall:?}")), "{err}");
        }
        assert!(load("hidden", wall).is_ok(), "hidden {wall}");
    }
    for fine in ["~/.cache", "~/.claude/projects", "/srv/app/.git/objects"] {
        assert!(load("writable", fine).is_ok(), "{fine}");
    }

    std::fs::write(
        loader.global_path(),
        "[defaults.sandbox]\nreadable = [\"~/.config\"]\n",
    )
    .unwrap();
    fx.repo_file("");
    assert!(loader.repo(fx.repo_path(), None).is_err());
}
