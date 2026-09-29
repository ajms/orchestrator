use std::path::PathBuf;

use orch_agent::{AgentAdapter, ClaudeCode};

fn agent_dirs(vars: &[(&str, &str)]) -> Vec<PathBuf> {
    let lookup = |key: &str| {
        vars.iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.to_string())
    };
    ClaudeCode::default().agent_dirs(&lookup)
}

#[test]
fn claudes_own_dirs_are_its_project_memory_and_plans() {
    assert_eq!(
        agent_dirs(&[("HOME", "/home/dev")]),
        [
            PathBuf::from("/home/dev/.claude/projects"),
            PathBuf::from("/home/dev/.claude/plans"),
        ]
    );
}

#[test]
fn claudes_config_dir_can_be_moved() {
    assert_eq!(
        agent_dirs(&[("HOME", "/home/dev"), ("CLAUDE_CONFIG_DIR", "/srv/claude")]),
        [
            PathBuf::from("/srv/claude/projects"),
            PathBuf::from("/srv/claude/plans"),
        ]
    );
}
