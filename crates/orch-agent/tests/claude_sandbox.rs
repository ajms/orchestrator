use std::path::PathBuf;

use orch_agent::{AgentAdapter, ClaudeCode, Projection, SandboxNeeds};

const PROFILE: &str = "/home/dev/.local/state/orchestrator/agent-profiles/claude";

fn needs(vars: &[(&str, &str)]) -> SandboxNeeds {
    let lookup = |key: &str| {
        vars.iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.to_string())
    };
    ClaudeCode::default().sandbox_needs(&lookup)
}

fn paths(paths: &[&str]) -> Vec<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

fn projected(needs: &SandboxNeeds, name: &str) -> Projection {
    let target = PathBuf::from(format!("{PROFILE}/{name}"));
    needs
        .projected
        .iter()
        .find(|projection| projection.target == target)
        .unwrap_or_else(|| panic!("{name} should be projected"))
        .clone()
}

#[test]
fn the_profile_is_a_throwaway_skeleton_with_only_its_state_dirs_persisting() {
    let needs = needs(&[("HOME", "/home/dev")]);
    assert_eq!(needs.scratch, paths(&[PROFILE]));
    assert_eq!(
        needs.writable,
        paths(&[
            &format!("{PROFILE}/projects"),
            &format!("{PROFILE}/todos"),
            &format!("{PROFILE}/shell-snapshots"),
            &format!("{PROFILE}/plans"),
        ])
    );
}

#[test]
fn only_the_users_credentials_are_projected_writable() {
    let needs = needs(&[("HOME", "/home/dev")]);
    let writable: Vec<_> = needs
        .projected
        .iter()
        .filter(|projection| projection.writable)
        .collect();
    assert_eq!(
        writable,
        [&Projection {
            source: "/home/dev/.claude/.credentials.json".into(),
            target: format!("{PROFILE}/.credentials.json").into(),
            writable: true,
        }]
    );
}

#[test]
fn an_agent_cannot_change_the_users_config_or_the_rendered_one() {
    let needs = needs(&[("HOME", "/home/dev")]);
    for name in [
        "settings.json",
        "settings.local.json",
        "hooks",
        "skills",
        "agents",
        "commands",
        "rules",
        "plugins",
        "CLAUDE.md",
    ] {
        let projection = projected(&needs, name);
        assert_eq!(
            projection.source,
            PathBuf::from(format!("/home/dev/.claude/{name}"))
        );
        assert!(!projection.writable, "{name} should be read-only");
    }
    let config = projected(&needs, ".claude.json");
    assert_eq!(config.source, config.target);
    assert!(!config.writable);
}

#[test]
fn the_host_sessions_and_config_file_never_enter_the_sandbox() {
    let needs = needs(&[("HOME", "/home/dev")]);
    let sources = needs.projected.iter().map(|p| &p.source);
    for path in needs.writable.iter().chain(sources) {
        assert_ne!(path, &PathBuf::from("/home/dev/.claude.json"));
        assert_ne!(path, &PathBuf::from("/home/dev/.claude"));
        assert_ne!(path, &PathBuf::from("/home/dev/.claude/projects"));
    }
}

#[test]
fn claude_reads_its_installation_and_never_updates_itself() {
    let needs = needs(&[("HOME", "/home/dev")]);
    assert_eq!(
        needs.read_only,
        paths(&["/home/dev/.local/share/claude", "/home/dev/.local/bin"])
    );
    assert_eq!(
        needs.env,
        [
            ("DISABLE_AUTOUPDATER".to_string(), "1".to_string()),
            ("CLAUDE_CONFIG_DIR".to_string(), PROFILE.to_string()),
        ]
    );
}

#[test]
fn claude_keeps_its_own_variables_such_as_its_api_key_and_config_dir() {
    let needs = needs(&[("HOME", "/home/dev")]);
    assert_eq!(needs.env_prefixes, ["ANTHROPIC_", "CLAUDE_"]);
}

#[test]
fn the_profile_lives_in_orchs_state_dir() {
    let needs = needs(&[("HOME", "/home/dev"), ("XDG_STATE_HOME", "/var/state")]);
    assert_eq!(
        needs.scratch,
        paths(&["/var/state/orchestrator/agent-profiles/claude"])
    );
}

#[test]
fn a_moved_config_dir_outside_home_is_hidden_with_its_sources_still_projected() {
    let moved = tempfile::tempdir().unwrap();
    let moved = moved.path().to_str().unwrap();
    let needs = needs(&[("HOME", "/home/dev"), ("CLAUDE_CONFIG_DIR", moved)]);
    assert_eq!(needs.hidden, paths(&[moved]));
    assert_eq!(
        projected(&needs, ".credentials.json").source,
        PathBuf::from(format!("{moved}/.credentials.json"))
    );
}

#[test]
fn a_config_dir_inside_home_needs_no_hiding() {
    assert!(needs(&[("HOME", "/home/dev")]).hidden.is_empty());
}

#[test]
fn the_real_config_stays_a_hard_wall() {
    let walls = needs(&[("HOME", "/home/dev")]).read_only_config();
    assert!(walls.contains(&PathBuf::from("/home/dev/.claude/settings.json")));
    assert!(walls.contains(&PathBuf::from("/home/dev/.claude/skills")));
    assert!(!walls.contains(&PathBuf::from("/home/dev/.claude/.credentials.json")));
}

#[test]
fn without_a_home_claude_needs_nothing() {
    assert_eq!(needs(&[]), SandboxNeeds::default());
}
