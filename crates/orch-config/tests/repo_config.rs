mod common;

use common::Fixture;
use orch_config::{ConfigError, ConfigProblem, REPO_FILE};

#[test]
fn repo_without_any_config_uses_built_in_defaults() {
    let fx = Fixture::new();
    let config = fx.approved();
    assert_eq!(config.setup_script().unwrap(), None);
    assert_eq!(config.teardown_script().unwrap(), None);
    assert_eq!(config.base_branch(), None);
    assert_eq!(config.default_preset(), None);
    assert_eq!(config.review_command(), None);
    assert_eq!(config.agent().unwrap().name, "claude");
    assert_eq!(config.agent().unwrap().binary, None);
    assert!(config.agent().unwrap().args.is_empty());
}

#[test]
fn repo_file_wins_over_global_default_and_personal_override_wins_over_both() {
    let fx = Fixture::new();
    fx.global(&format!(
        "[defaults]\npreset = \"ask\"\nbase = \"develop\"\nreview_command = \"delta\"\n\n{}",
        fx.personal("base = \"trunk\"\n")
    ));
    fx.repo_file("base = \"main\"\npreset = \"plan\"\nsetup = \"make deps\"\n");

    let config = fx.approved();
    assert_eq!(config.base_branch(), Some("trunk"));
    assert_eq!(config.default_preset(), Some("plan"));
    assert_eq!(config.review_command(), Some("delta"));
    assert_eq!(config.setup_script().unwrap(), Some("make deps"));
}

#[test]
fn agent_binary_and_args_come_from_the_highest_layer_that_sets_them() {
    let fx = Fixture::new();
    fx.global(&format!(
        "[repos.{}.agent]\nbinary = \"/opt/claude/bin/claude\"\n",
        fx.repo_key()
    ));
    fx.repo_file("[agent]\nname = \"claude\"\nargs = [\"--verbose\"]\n");

    let config = fx.approved();
    assert_eq!(
        config.agent().unwrap().binary.as_deref(),
        Some("/opt/claude/bin/claude")
    );
    assert_eq!(config.agent().unwrap().args, vec!["--verbose".to_string()]);
}

#[test]
fn repo_config_is_re_read_on_every_use() {
    let fx = Fixture::new();
    fx.repo_file("setup = \"one\"\n");
    assert_eq!(fx.approved().setup_script().unwrap(), Some("one"));
    fx.repo_file("setup = \"two\"\n");
    assert_eq!(fx.approved().setup_script().unwrap(), Some("two"));
}

#[test]
fn global_only_settings_in_the_repo_file_are_rejected() {
    let fx = Fixture::new();
    fx.repo_file("branch_prefix = \"x/\"\n");
    let err = fx.loader.repo(fx.repo_path(), None).unwrap_err();
    assert!(err.to_string().contains(REPO_FILE), "{err}");
}

#[test]
fn review_command_is_personal_and_rejected_in_the_repo_file() {
    let fx = Fixture::new();
    fx.repo_file("review_command = \"./tools/diff\"\n");
    assert!(matches!(
        fx.loader.repo(fx.repo_path(), None),
        Err(ConfigError::Invalid {
            problem: ConfigProblem::Misplaced {
                key: "review_command"
            },
            ..
        })
    ));

    fx.repo_file("");
    fx.global(&fx.personal("review_command = \"nvim -d\"\n"));
    assert_eq!(fx.untrusted().review_command(), Some("nvim -d"));
}

#[test]
fn personal_override_keys_may_start_with_a_tilde() {
    let fx = Fixture::new();
    let loader = fx
        .loader
        .clone()
        .with_home(fx.dir.path().canonicalize().unwrap());
    std::fs::write(
        loader.global_path(),
        "[repos.\"~/repo\"]\nbase = \"trunk\"\n",
    )
    .unwrap();
    assert_eq!(
        loader.repo(fx.repo_path(), None).unwrap().base_branch(),
        Some("trunk")
    );
}

#[test]
fn override_keys_pointing_at_a_path_are_listed_even_after_it_is_gone() {
    let fx = Fixture::new();
    let loader = fx
        .loader
        .clone()
        .with_home(fx.dir.path().canonicalize().unwrap());
    let old = fx.repo.clone();
    std::fs::write(
        loader.global_path(),
        format!(
            "[repos.\"~/repo\"]\nbase = \"a\"\n[repos.{:?}]\nsetup = \"b\"\n[repos.\"/elsewhere\"]\nbase = \"c\"\n",
            old.display().to_string()
        ),
    )
    .unwrap();
    std::fs::remove_dir_all(&old).unwrap();
    assert_eq!(
        loader.override_keys_for(&old).unwrap(),
        vec![old.display().to_string(), "~/repo".to_string()]
    );
}

#[test]
fn personal_override_keys_match_non_canonical_spellings_of_the_repo_path() {
    let fx = Fixture::new();
    let sneaky = fx.repo.join("..").join("repo");
    fx.global(&format!(
        "[repos.{:?}]\nbase = \"trunk\"\n",
        sneaky.display().to_string()
    ));
    assert_eq!(fx.approved().base_branch(), Some("trunk"));
}
