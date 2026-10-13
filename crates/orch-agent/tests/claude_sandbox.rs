use std::path::PathBuf;

use orch_agent::{AgentAdapter, ClaudeCode, SandboxNeeds};

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
fn claude_writes_its_config_dir_and_config_file_but_not_its_settings() {
    let needs = needs(&[("HOME", "/home/dev")]);
    assert_eq!(
        needs.writable,
        paths(&["/home/dev/.claude", "/home/dev/.claude.json"])
    );
    assert_eq!(
        needs.carve_outs,
        paths(&[
            "/home/dev/.claude/settings.json",
            "/home/dev/.claude/settings.local.json",
            "/home/dev/.claude/hooks",
            "/home/dev/.claude/skills",
            "/home/dev/.claude/agents",
            "/home/dev/.claude/commands",
            "/home/dev/.claude/rules",
            "/home/dev/.claude/plugins",
            "/home/dev/.claude/CLAUDE.md",
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
        [("DISABLE_AUTOUPDATER".to_string(), "1".to_string())]
    );
}

#[test]
fn a_moved_config_dir_is_writable_with_its_settings_carved_out() {
    let needs = needs(&[("HOME", "/home/dev"), ("CLAUDE_CONFIG_DIR", "/srv/claude")]);
    assert_eq!(
        needs.writable,
        paths(&["/srv/claude", "/home/dev/.claude.json"])
    );
    assert!(
        needs
            .carve_outs
            .contains(&PathBuf::from("/srv/claude/hooks"))
    );
    assert!(
        needs
            .carve_outs
            .iter()
            .all(|path| path.starts_with("/srv/claude"))
    );
}
