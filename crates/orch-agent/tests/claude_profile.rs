use std::path::{Path, PathBuf};

use orch_agent::{AgentAdapter, ClaudeCode, ProfileRender};
use orch_core::ConversationId;
use serde_json::{Value, json};
use tempfile::TempDir;

const WORKTREE: &str = "/work/my.tree";
const PROJECT: &str = "-work-my-tree";
const CONVERSATION: &str = "11111111-1111-4111-8111-111111111111";

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
        self.launch(worktree, None);
    }

    fn resume(&self, conversation: &str) {
        self.launch(WORKTREE, Some(&ConversationId(conversation.into())));
    }

    fn launch(&self, worktree: &str, conversation: Option<&ConversationId>) {
        let render = ProfileRender {
            env: &[],
            worktree: Path::new(worktree),
            conversation,
        };
        ClaudeCode::default()
            .render_profile(&self.lookup(), &render)
            .unwrap();
    }

    fn project_in(&self, dir: &Path) -> PathBuf {
        dir.join("projects").join(PROJECT)
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
    for name in ["projects", "todos", "shell-snapshots", "plans"] {
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
    std::fs::write(home.profile().join("projects/live.jsonl"), "turns").unwrap();
    home.prepare();
    assert!(home.profile().join("projects/live.jsonl").exists());
}

#[test]
fn trust_survives_a_parallel_launch_and_a_rebuild() {
    let home = Home::new();
    home.prepare();
    home.render("/work/one");
    home.render("/work/two");
    home.prepare();
    let projects = home.config()["projects"].clone();
    assert_eq!(projects["/work/one"]["hasTrustDialogAccepted"], true);
    assert_eq!(projects["/work/two"]["hasTrustDialogAccepted"], true);
}

#[test]
fn a_rebuild_keeps_only_the_config_orch_owns() {
    let home = Home::new();
    home.prepare();
    let planted = json!({
        "mcpServers": { "planted": {} },
        "projects": {
            "/a": { "hasTrustDialogAccepted": true, "mcpServers": { "planted": {} } },
            "/b": { "hasTrustDialogAccepted": false },
        },
    });
    home.seed(&home.profile().join(".claude.json"), &planted.to_string());
    home.prepare();
    let config = home.config();
    assert!(config["mcpServers"].is_null());
    assert_eq!(
        config["projects"]["/a"],
        json!({ "hasTrustDialogAccepted": true })
    );
    assert!(config["projects"]["/b"].is_null());
}

#[test]
fn a_malformed_config_never_stops_a_launch() {
    let home = Home::new();
    home.prepare();
    let config = home.profile().join(".claude.json");
    home.seed(&config, r#"{"projects":"nope"}"#);
    home.render("/work/tree");
    assert_eq!(
        home.config()["projects"]["/work/tree"]["hasTrustDialogAccepted"],
        true
    );
}

#[test]
fn a_linked_config_is_never_followed() {
    let home = Home::new();
    home.prepare();
    let foreign = home.path().join("foreign.json");
    std::fs::write(&foreign, r#"{"foreign":true}"#).unwrap();
    let config = home.profile().join(".claude.json");
    std::fs::remove_file(&config).unwrap();
    std::os::unix::fs::symlink(&foreign, &config).unwrap();

    home.prepare();

    assert!(!config.symlink_metadata().unwrap().file_type().is_symlink());
    assert!(home.config()["foreign"].is_null());
    assert_eq!(
        std::fs::read_to_string(&foreign).unwrap(),
        r#"{"foreign":true}"#
    );
}

#[test]
fn a_rebuild_resets_a_replaced_carve_out() {
    let home = Home::new();
    home.prepare();
    let settings = home.profile().join("settings.json");
    std::fs::remove_file(&settings).unwrap();
    std::fs::write(&settings, "planted").unwrap();

    home.prepare();

    assert_eq!(
        std::fs::read_link(&settings).unwrap(),
        home.user().join("settings.json")
    );
}

#[test]
fn resuming_adopts_only_that_conversation_from_before_the_profile() {
    let home = Home::new();
    let source = home.project_in(&home.user());
    home.seed(&source.join(format!("{CONVERSATION}.jsonl")), "old turns");
    let nested = format!("{CONVERSATION}/subagents/one.jsonl");
    home.seed(&source.join(&nested), "sub");
    home.seed(&source.join("other.jsonl"), "another conversation");
    home.prepare();

    home.resume(CONVERSATION);

    let adopted = home.project_in(&home.profile());
    let read = |name: &str| std::fs::read_to_string(adopted.join(name)).unwrap();
    assert_eq!(read(&format!("{CONVERSATION}.jsonl")), "old turns");
    assert_eq!(read(&nested), "sub");
    assert!(!adopted.join("other.jsonl").exists());
}

#[test]
fn adopting_never_overwrites_the_profiles_newer_conversation() {
    let home = Home::new();
    let file = format!("{CONVERSATION}.jsonl");
    home.seed(&home.project_in(&home.user()).join(&file), "old");
    home.prepare();
    let adopted = home.project_in(&home.profile()).join(&file);
    home.seed(&adopted, "newer");

    home.resume(CONVERSATION);

    assert_eq!(std::fs::read_to_string(adopted).unwrap(), "newer");
}

#[test]
fn adopting_never_writes_through_a_planted_link() {
    let home = Home::new();
    let file = format!("{CONVERSATION}.jsonl");
    home.seed(&home.project_in(&home.user()).join(&file), "old");
    home.prepare();
    let elsewhere = home.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, home.project_in(&home.profile())).unwrap();

    home.resume(CONVERSATION);

    assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
    assert!(home.project_in(&home.profile()).join(&file).is_file());
}

#[test]
fn a_new_conversation_has_nothing_to_adopt() {
    let home = Home::new();
    home.prepare();
    home.resume(CONVERSATION);
    assert!(!home.project_in(&home.profile()).exists());
}

#[test]
fn a_conversation_id_that_is_a_path_adopts_nothing() {
    let home = Home::new();
    home.seed(&home.path().join("outside.jsonl"), "outside");
    home.seed(&home.project_in(&home.user()).join("seed.jsonl"), "");
    home.prepare();

    home.resume("../../../outside");

    assert!(!home.project_in(&home.profile()).exists());
}
