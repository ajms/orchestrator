use std::path::PathBuf;

use orch_agent::{AgentAdapter, ClaudeCode, SandboxNeeds};

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

#[test]
fn claude_writes_only_its_profile_and_the_users_credentials() {
    let needs = needs(&[("HOME", "/home/dev")]);
    assert_eq!(
        needs.writable,
        paths(&[PROFILE, "/home/dev/.claude/.credentials.json"])
    );
}

#[test]
fn the_host_sessions_and_config_file_never_enter_the_sandbox() {
    let needs = needs(&[("HOME", "/home/dev")]);
    assert!(
        !needs
            .writable
            .contains(&PathBuf::from("/home/dev/.claude.json"))
    );
    assert!(!needs.writable.contains(&PathBuf::from("/home/dev/.claude")));
}

#[test]
fn an_agent_cannot_plant_config_in_the_profile() {
    let needs = needs(&[("HOME", "/home/dev")]);
    assert_eq!(
        needs.carve_outs,
        paths(&[
            &format!("{PROFILE}/settings.json"),
            &format!("{PROFILE}/settings.local.json"),
            &format!("{PROFILE}/hooks"),
            &format!("{PROFILE}/skills"),
            &format!("{PROFILE}/agents"),
            &format!("{PROFILE}/commands"),
            &format!("{PROFILE}/rules"),
            &format!("{PROFILE}/plugins"),
            &format!("{PROFILE}/CLAUDE.md"),
        ])
    );
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
    assert!(needs.writable.contains(&PathBuf::from(
        "/var/state/orchestrator/agent-profiles/claude"
    )));
}

#[test]
fn the_profile_links_credentials_from_a_moved_config_dir() {
    let needs = needs(&[("HOME", "/home/dev"), ("CLAUDE_CONFIG_DIR", "/srv/claude")]);
    assert_eq!(
        needs.writable,
        paths(&[PROFILE, "/srv/claude/.credentials.json"])
    );
}

#[test]
fn without_a_home_claude_needs_nothing() {
    assert_eq!(needs(&[]), SandboxNeeds::default());
}
