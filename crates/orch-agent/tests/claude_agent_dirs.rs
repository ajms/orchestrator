use std::path::PathBuf;

use orch_agent::{AgentAdapter, ClaudeCode};

const PROFILE: &str = "/home/dev/.local/state/orchestrator/agent-profiles/claude";

fn agent_dirs(vars: &[(&str, &str)]) -> Vec<PathBuf> {
    let lookup = |key: &str| {
        vars.iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.to_string())
    };
    ClaudeCode::default().agent_dirs(&lookup)
}

#[test]
fn claudes_own_dirs_are_its_project_memory_and_plans_in_the_profile() {
    assert_eq!(
        agent_dirs(&[("HOME", "/home/dev")]),
        [
            PathBuf::from(format!("{PROFILE}/projects")),
            PathBuf::from(format!("{PROFILE}/plans")),
        ]
    );
}

#[test]
fn the_profile_follows_orchs_state_dir() {
    assert_eq!(
        agent_dirs(&[("HOME", "/home/dev"), ("XDG_STATE_HOME", "/var/state")]),
        [
            PathBuf::from("/var/state/orchestrator/agent-profiles/claude/projects"),
            PathBuf::from("/var/state/orchestrator/agent-profiles/claude/plans"),
        ]
    );
}
