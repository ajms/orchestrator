use orch_agent::{AgentAdapter, Argv, Capabilities, ClaudeCode, LaunchSpec};
use tempfile::TempDir;

struct Layout {
    dir: TempDir,
}

impl Layout {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("worktree")).unwrap();
        Self { dir }
    }

    fn path(&self, rel: &str) -> std::path::PathBuf {
        self.dir.path().join(rel)
    }

    fn statusline(&self, rel: &str, command: &str) {
        let path = self.path(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let settings =
            serde_json::json!({ "statusLine": { "type": "command", "command": command } });
        std::fs::write(path, settings.to_string()).unwrap();
    }

    fn lookup(&self, extra: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let mut vars: Vec<(String, String)> = vec![
            ("HOME".into(), self.path("home").display().to_string()),
            (
                "ORCH_CLAUDE_MANAGED_SETTINGS".into(),
                self.path("managed/managed-settings.json")
                    .display()
                    .to_string(),
            ),
        ];
        vars.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        move |key| vars.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    fn resolve(&self, extra: &[(&str, &str)]) -> Option<String> {
        ClaudeCode::default().user_statusline_command(&self.path("worktree"), self.lookup(extra))
    }
}

#[test]
fn no_settings_means_no_statusline() {
    assert_eq!(Layout::new().resolve(&[]), None);
}

#[test]
fn user_settings_supply_the_statusline() {
    let layout = Layout::new();
    layout.statusline("home/.claude/settings.json", "user-line");

    assert_eq!(layout.resolve(&[]).as_deref(), Some("user-line"));
}

#[test]
fn claude_config_dir_replaces_the_home_settings() {
    let layout = Layout::new();
    layout.statusline("home/.claude/settings.json", "home-line");
    layout.statusline("config/settings.json", "config-dir-line");
    let config = layout.path("config").display().to_string();

    assert_eq!(
        layout.resolve(&[("CLAUDE_CONFIG_DIR", &config)]).as_deref(),
        Some("config-dir-line")
    );
}

#[test]
fn project_settings_override_user_settings() {
    let layout = Layout::new();
    layout.statusline("home/.claude/settings.json", "user-line");
    layout.statusline("worktree/.claude/settings.json", "project-line");

    assert_eq!(layout.resolve(&[]).as_deref(), Some("project-line"));
}

#[test]
fn local_project_settings_override_shared_project_settings() {
    let layout = Layout::new();
    layout.statusline("worktree/.claude/settings.json", "project-line");
    layout.statusline("worktree/.claude/settings.local.json", "local-line");

    assert_eq!(layout.resolve(&[]).as_deref(), Some("local-line"));
}

#[test]
fn managed_settings_override_everything() {
    let layout = Layout::new();
    layout.statusline("worktree/.claude/settings.local.json", "local-line");
    layout.statusline("managed/managed-settings.json", "managed-line");

    assert_eq!(layout.resolve(&[]).as_deref(), Some("managed-line"));
}

#[test]
fn unreadable_or_commandless_settings_are_skipped() {
    let layout = Layout::new();
    layout.statusline("home/.claude/settings.json", "user-line");
    std::fs::create_dir_all(layout.path("worktree/.claude")).unwrap();
    std::fs::write(
        layout.path("worktree/.claude/settings.local.json"),
        "{ not json",
    )
    .unwrap();
    std::fs::write(
        layout.path("worktree/.claude/settings.json"),
        r#"{"statusLine":{"type":"command","command":"  "}}"#,
    )
    .unwrap();

    assert_eq!(layout.resolve(&[]).as_deref(), Some("user-line"));
}

#[test]
fn only_pretooluse_payloads_wait_for_a_guard_answer() {
    let claude = ClaudeCode::default();
    assert!(claude.is_guard_payload(r#"{"hook_event_name":"PreToolUse","tool_name":"Bash"}"#));
    assert!(!claude.is_guard_payload(r#"{"hook_event_name":"PostToolUse","tool_name":"Bash"}"#));
    assert!(!claude.is_guard_payload("not json"));
}

struct NoHooks;

impl AgentAdapter for NoHooks {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn launch(&self, _spec: &LaunchSpec) -> Argv {
        Argv {
            program: "none".into(),
            args: vec![],
        }
    }
}

#[test]
fn adapters_without_guards_never_wait_for_an_answer() {
    assert!(!NoHooks.is_guard_payload(r#"{"hook_event_name":"PreToolUse"}"#));
}
