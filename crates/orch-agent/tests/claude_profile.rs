use std::path::{Path, PathBuf};

use orch_agent::{AgentAdapter, ClaudeCode};
use serde_json::Value;
use tempfile::TempDir;

struct Home {
    dir: TempDir,
}

impl Home {
    fn new() -> Self {
        let home = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        for (path, content) in [
            (".credentials.json", r#"{"claudeAiOauth":{}}"#),
            ("settings.json", r#"{"theme":"dark"}"#),
            ("CLAUDE.md", "house rules"),
            ("hooks/pre.sh", "echo hi"),
            // Host state that must never enter the profile.
            ("projects/foreign/session.jsonl", "secret history"),
            (".claude.json", r#"{"projects":{"/elsewhere":{}}}"#),
            ("history.jsonl", "old"),
            ("statsig/evaluations", "cache"),
        ] {
            home.seed(&home.user().join(path), content);
        }
        home
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn user(&self) -> PathBuf {
        self.path().join(".claude")
    }

    fn profile(&self) -> PathBuf {
        self.path()
            .join(".local/state/orchestrator/agent-profiles/claude")
    }

    fn seed(&self, path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn lookup(&self) -> impl Fn(&str) -> Option<String> {
        let home = self.path().to_string_lossy().into_owned();
        move |key: &str| (key == "HOME").then(|| home.clone())
    }

    fn prepare(&self) {
        ClaudeCode::default().prepare(&self.lookup()).unwrap();
    }

    fn render(&self, worktree: &str) {
        ClaudeCode::default()
            .render_profile(
                &self.lookup(),
                &[("ORCH_WORKTREE".to_string(), worktree.to_string())],
            )
            .unwrap();
    }

    fn config(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.profile().join(".claude.json")).unwrap())
            .unwrap()
    }
}

#[test]
fn credentials_are_linked_not_copied() {
    let home = Home::new();
    home.prepare();
    let link = home.profile().join(".credentials.json");
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        home.user().join(".credentials.json")
    );
}

#[test]
fn the_config_carve_outs_are_linked_read_only() {
    let home = Home::new();
    home.prepare();
    for name in ["settings.json", "CLAUDE.md", "hooks"] {
        assert_eq!(
            std::fs::read_link(home.profile().join(name)).unwrap(),
            home.user().join(name),
            "{name} should be linked from the real ~/.claude",
        );
    }
}

#[test]
fn the_writable_state_dirs_are_created_empty() {
    let home = Home::new();
    home.prepare();
    for name in ["projects", "todos", "shell-snapshots"] {
        let dir = home.profile().join(name);
        assert!(dir.is_dir(), "{name} should be a writable dir");
        assert!(!dir.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    }
}

#[test]
fn the_host_sessions_history_and_config_never_enter_the_profile() {
    let home = Home::new();
    home.prepare();
    // projects/ exists but is orch's own empty dir, not a link to the host's.
    assert!(
        !home
            .profile()
            .join("projects")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!home.profile().join("projects/foreign").exists());
    assert!(!home.profile().join("history.jsonl").exists());
    assert!(!home.profile().join("statsig").exists());
    // The rendered .claude.json is synthesized, not the host's.
    assert!(home.config()["projects"]["/elsewhere"].is_null());
}

#[test]
fn prepare_seeds_onboarding_without_a_worktree() {
    let home = Home::new();
    home.prepare();
    assert_eq!(home.config()["hasCompletedOnboarding"], Value::Bool(true));
    assert!(
        home.config()["projects"]
            .as_object()
            .is_none_or(|p| p.is_empty())
    );
}

#[test]
fn render_pre_trusts_the_session_worktree() {
    let home = Home::new();
    home.prepare();
    home.render("/work/tree");
    let config = home.config();
    assert_eq!(config["hasCompletedOnboarding"], Value::Bool(true));
    assert_eq!(
        config["projects"]["/work/tree"]["hasTrustDialogAccepted"],
        Value::Bool(true)
    );
}

#[test]
fn a_rebuild_keeps_the_writable_state() {
    let home = Home::new();
    home.prepare();
    home.render("/work/tree");
    std::fs::write(home.profile().join("projects/live.jsonl"), "transcript").unwrap();
    // A second launch rebuilds the profile but must keep the transcripts.
    home.prepare();
    assert!(home.profile().join("projects/live.jsonl").exists());
}
