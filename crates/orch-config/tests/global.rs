use std::time::Duration;

use orch_config::{
    ConfigError, ConfigLoader, ConfigProblem, PortBlock, PortRange, ReadableOwners, is_github_owner,
};
use tempfile::TempDir;

fn loader_in(dir: &TempDir) -> ConfigLoader {
    ConfigLoader::new(dir.path().join("config.toml"))
}

#[test]
fn missing_global_file_yields_defaults() {
    let dir = TempDir::new().unwrap();
    let global = loader_in(&dir).global().unwrap();
    assert_eq!(global.branch_prefix, "orch/");
    assert_eq!(global.stalled_after, Duration::from_secs(600));
    assert_eq!(
        global.ports,
        PortRange {
            start: 20000,
            end: 29999,
            block_size: 10
        }
    );
}

#[test]
fn global_file_is_re_read_on_every_use() {
    let dir = TempDir::new().unwrap();
    let loader = loader_in(&dir);
    std::fs::write(loader.global_path(), "branch_prefix = \"work/\"\n").unwrap();
    assert_eq!(loader.global().unwrap().branch_prefix, "work/");

    std::fs::write(
        loader.global_path(),
        "branch_prefix = \"agent/\"\nstalled_minutes = 3\n[ports]\nstart = 4000\nend = 4099\nblock_size = 20\n",
    )
    .unwrap();
    let global = loader.global().unwrap();
    assert_eq!(global.branch_prefix, "agent/");
    assert_eq!(global.stalled_after, Duration::from_secs(180));
    assert_eq!(
        global.ports,
        PortRange {
            start: 4000,
            end: 4099,
            block_size: 20
        }
    );
}

#[test]
fn malformed_global_file_is_an_error_naming_the_file() {
    let dir = TempDir::new().unwrap();
    let loader = loader_in(&dir);
    std::fs::write(loader.global_path(), "branch_prefix = [").unwrap();
    let err = loader.global().unwrap_err();
    assert!(err.to_string().contains("config.toml"), "{err}");
}

#[test]
fn port_range_without_room_for_a_block_is_rejected() {
    let dir = TempDir::new().unwrap();
    let loader = loader_in(&dir);
    std::fs::write(
        loader.global_path(),
        "[ports]\nstart = 5000\nend = 5004\nblock_size = 10\n",
    )
    .unwrap();
    assert!(matches!(
        loader.global(),
        Err(ConfigError::Invalid {
            problem: ConfigProblem::EmptyPortRange(_),
            ..
        })
    ));
}

#[test]
fn a_port_range_is_divided_into_whole_blocks() {
    let range = PortRange {
        start: 4000,
        end: 4024,
        block_size: 10,
    };
    assert_eq!(
        range.blocks().collect::<Vec<_>>(),
        vec![
            PortBlock {
                base: 4000,
                size: 10
            },
            PortBlock {
                base: 4010,
                size: 10
            },
        ]
    );
    assert!(
        PortBlock {
            base: 4005,
            size: 10
        }
        .overlaps(PortBlock {
            base: 4014,
            size: 1
        })
    );
    assert!(
        !PortBlock {
            base: 4005,
            size: 10
        }
        .overlaps(PortBlock {
            base: 4015,
            size: 1
        })
    );
}

#[test]
fn global_path_follows_xdg_config_home_then_home() {
    let xdg = ConfigLoader::from_env_vars(|key| match key {
        "XDG_CONFIG_HOME" => Some("/x/config".into()),
        "HOME" => Some("/home/u".into()),
        _ => None,
    });
    assert_eq!(
        xdg.unwrap().global_path(),
        std::path::Path::new("/x/config/orchestrator/config.toml")
    );

    let home = ConfigLoader::from_env_vars(|key| match key {
        "XDG_CONFIG_HOME" => Some(String::new()),
        "HOME" => Some("/home/u".into()),
        _ => None,
    });
    assert_eq!(
        home.unwrap().global_path(),
        std::path::Path::new("/home/u/.config/orchestrator/config.toml")
    );

    assert!(ConfigLoader::from_env_vars(|_| None).is_none());
}

#[test]
fn the_agent_signing_key_is_unset_by_default_and_expands_a_home_path() {
    let dir = TempDir::new().unwrap();
    let loader = loader_in(&dir).with_home("/home/u");
    assert_eq!(loader.global().unwrap().signing_key, None);

    std::fs::write(loader.global_path(), "[signing]\nkey = \"~/keys/orch\"\n").unwrap();
    assert_eq!(
        loader.global().unwrap().signing_key,
        Some(std::path::PathBuf::from("/home/u/keys/orch"))
    );
}

#[test]
fn the_broker_reads_no_other_owners_by_default_and_those_listed_once_configured() {
    let dir = TempDir::new().unwrap();
    let loader = loader_in(&dir);
    assert!(!loader.global().unwrap().readable_owners.reads("ajms"));

    std::fs::write(
        loader.global_path(),
        "[broker]\nreadable_owners = [\"ajms\", \"Acme-Org\"]\n",
    )
    .unwrap();
    let owners = loader.global().unwrap().readable_owners;
    assert!(owners.reads("ajms") && owners.reads("acme-org") && owners.reads("ACME-ORG"));
    assert!(!owners.reads("acme"));
}

#[test]
fn a_readable_owner_is_spelled_as_written_and_in_lower_case() {
    let owners = ReadableOwners::new(vec!["Acme-Org".into(), "ajms".into()]).unwrap();
    assert_eq!(owners.spellings(), ["Acme-Org", "acme-org", "ajms"]);
}

#[test]
fn a_readable_owner_must_be_a_plain_github_owner() {
    let dir = TempDir::new().unwrap();
    let loader = loader_in(&dir);
    for owner in ["acme/app", "", "../x", "acme org", "*"] {
        std::fs::write(
            loader.global_path(),
            format!("[broker]\nreadable_owners = [{owner:?}]\n"),
        )
        .unwrap();
        assert!(loader.global().is_err(), "{owner:?} was accepted");
    }
    assert!(is_github_owner("Acme-Org1"));
    assert!(!is_github_owner("-acme") && !is_github_owner("acme_org"));
}
