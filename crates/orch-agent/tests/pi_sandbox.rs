use std::path::PathBuf;

use orch_agent::{AgentAdapter, Pi, SandboxNeeds};

const PROFILE: &str = "/home/dev/.local/state/orchestrator/agent-profiles/pi";

fn lookup(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
    move |key: &str| {
        vars.iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.to_string())
    }
}

fn needs() -> SandboxNeeds {
    Pi::default().sandbox_needs(&lookup(&[("HOME", "/home/dev"), ("PI_TOKEN", "secret")]))
}

fn paths(paths: &[&str]) -> Vec<PathBuf> {
    paths.iter().map(PathBuf::from).collect()
}

fn pairs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[test]
fn every_launch_points_pi_at_its_agent_profile_offline() {
    let env = Pi::default().env(&lookup(&[("HOME", "/home/dev")]));
    assert_eq!(
        env,
        pairs(&[
            ("PI_CODING_AGENT_DIR", PROFILE),
            ("PI_OFFLINE", "1"),
            ("PI_INDEX_ROOT", &format!("{PROFILE}/index")),
        ])
    );
}

#[test]
fn the_agent_profile_lives_in_orchs_state_dir() {
    let env = Pi::default().env(&lookup(&[
        ("HOME", "/home/dev"),
        ("XDG_STATE_HOME", "/var/state"),
    ]));
    assert_eq!(
        env[0],
        (
            "PI_CODING_AGENT_DIR".to_string(),
            "/var/state/orchestrator/agent-profiles/pi".to_string()
        )
    );
}

#[test]
fn the_sandbox_passes_only_the_variables_orch_sets_for_pi() {
    let needs = needs();
    assert_eq!(
        needs.env,
        Pi::default().env(&lookup(&[("HOME", "/home/dev")]))
    );
    assert!(needs.env_prefixes.is_empty());
}

#[test]
fn pi_writes_its_profile_and_the_users_credentials() {
    assert_eq!(
        needs().writable,
        paths(&[
            PROFILE,
            "/home/dev/.pi/agent/auth.json",
            "/home/dev/.pi/agent/models-store.json",
        ])
    );
}

#[test]
fn pi_reads_its_installation_and_the_users_setup_but_not_their_sessions() {
    assert_eq!(
        needs().read_only,
        paths(&[
            "/home/dev/.pi/agent/install",
            "/home/dev/.pi/agent/bin",
            "/home/dev/.pi/agent/extensions",
            "/home/dev/.pi/agent/skills",
            "/home/dev/.pi/agent/prompts",
            "/home/dev/.pi/agent/themes",
            "/home/dev/.pi/agent/agents",
            "/home/dev/.pi/agent/SYSTEM.md",
            "/home/dev/.pi/agent/AGENTS.md",
            "/home/dev/.pi/agent/keybindings.json",
            "/home/dev/.pi/agent/settings.json",
            "/home/dev/.pi/agent/sandbox.json",
            "/home/dev/.local/share/pi-node",
            "/home/dev/.local/bin",
        ])
    );
}

#[test]
fn an_agent_cannot_plant_resources_in_the_profile() {
    assert_eq!(
        needs().carve_outs,
        paths(&[
            &format!("{PROFILE}/extensions"),
            &format!("{PROFILE}/skills"),
            &format!("{PROFILE}/prompts"),
            &format!("{PROFILE}/themes"),
        ])
    );
}

#[test]
fn without_a_home_pi_needs_nothing() {
    assert_eq!(
        Pi::default().sandbox_needs(&lookup(&[])),
        SandboxNeeds::default()
    );
}
