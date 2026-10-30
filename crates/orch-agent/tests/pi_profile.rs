use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use orch_agent::{AgentAdapter, Pi};
use serde_json::{Value, json};
use tempfile::TempDir;

const CREDENTIALS: &str = "auth.json";
const MODELS: &str = "models-store.json";

struct Home {
    dir: TempDir,
}

impl Home {
    fn new() -> Self {
        let home = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        for (path, content) in [
            (
                "settings.json",
                r#"{"theme":"dark","sessionDir":"/elsewhere","extensions":["mine/index.ts","/opt/ext","!old/**"]}"#,
            ),
            ("SYSTEM.md", "be brief"),
            ("AGENTS.md", "house rules"),
            ("keybindings.json", "{}"),
            (CREDENTIALS, r#"{"anthropic":{}}"#),
            (MODELS, "{}"),
            ("agents/explorer.md", "explore"),
            ("extensions/notify/index.ts", ""),
            ("sessions/--home--/own.jsonl", "mine"),
        ] {
            home.seed(&home.user().join(path), content);
        }
        home
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn user(&self) -> PathBuf {
        self.path().join(".pi/agent")
    }

    fn profile(&self) -> PathBuf {
        self.path()
            .join(".local/state/orchestrator/agent-profiles/pi")
    }

    fn seed(&self, path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn prepare(&self) {
        let home = self.path().to_string_lossy().into_owned();
        let lookup = move |key: &str| (key == "HOME").then(|| home.clone());
        Pi::default()
            .prepare(&lookup)
            .expect("the profile is built");
    }

    fn settings(&self) -> Value {
        serde_json::from_str(
            &std::fs::read_to_string(self.profile().join("settings.json")).unwrap(),
        )
        .unwrap()
    }
}

fn snapshot(dir: &Path) -> BTreeMap<PathBuf, String> {
    let mut found = BTreeMap::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        let meta = path.symlink_metadata().unwrap();
        if meta.is_dir() {
            found.extend(snapshot(&path));
        } else {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            found.insert(path, format!("{content} @ {:?}", meta.modified().unwrap()));
        }
    }
    found.insert(
        dir.into(),
        format!("{:?}", dir.metadata().unwrap().modified().unwrap()),
    );
    found
}

#[test]
fn settings_layer_the_users_resources_minus_the_ones_orch_owns() {
    let home = Home::new();
    home.prepare();

    let user = home.user().to_string_lossy().into_owned();
    let excluded = |name: &str| format!("-{user}/extensions/{name}/index.ts");
    assert_eq!(
        home.settings()["extensions"],
        json!([
            format!("{user}/extensions"),
            format!("{user}/mine/index.ts"),
            "/opt/ext",
            "!old/**",
            excluded("git-worktree-isolation"),
            excluded("sandbox"),
            excluded("notify"),
            excluded("caffeinate"),
        ])
    );
    for kind in ["skills", "prompts", "themes"] {
        assert_eq!(home.settings()[kind], json!([format!("{user}/{kind}")]));
    }
}

#[test]
fn settings_keep_the_users_choices_but_not_their_session_dir() {
    let home = Home::new();
    home.prepare();

    assert_eq!(home.settings()["theme"], "dark");
    assert_eq!(home.settings().get("sessionDir"), None);
}

#[test]
fn without_user_settings_the_profile_still_lists_the_users_resources() {
    let home = Home::new();
    std::fs::remove_file(home.user().join("settings.json")).unwrap();
    home.prepare();

    let user = home.user().to_string_lossy().into_owned();
    assert_eq!(home.settings()["skills"], json!([format!("{user}/skills")]));
}

#[test]
fn the_users_prompts_keys_and_credentials_are_linked_into_the_profile() {
    let home = Home::new();
    home.prepare();

    for name in [
        "SYSTEM.md",
        "AGENTS.md",
        "keybindings.json",
        "agents",
        CREDENTIALS,
        MODELS,
    ] {
        assert_eq!(
            std::fs::read_link(home.profile().join(name)).unwrap(),
            home.user().join(name),
            "{name}"
        );
    }
}

#[test]
fn a_file_the_user_lacks_is_not_linked() {
    let home = Home::new();
    std::fs::remove_file(home.user().join("SYSTEM.md")).unwrap();
    home.prepare();

    assert!(home.profile().join("SYSTEM.md").symlink_metadata().is_err());
}

#[test]
fn the_profile_has_empty_resource_dirs_and_none_of_the_users_sessions() {
    let home = Home::new();
    home.prepare();

    for kind in ["extensions", "skills", "prompts", "themes"] {
        let dir = home.profile().join(kind);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "{kind}");
    }
    assert!(!home.profile().join("sessions/--home--").exists());
}

#[test]
fn a_new_launch_resets_what_an_agent_changed_but_keeps_its_conversations() {
    let home = Home::new();
    home.prepare();
    let profile = home.profile();
    let conversation = profile.join("sessions/--work--/2026_s-1.jsonl");
    home.seed(&conversation, "turns");
    home.seed(
        &profile.join("settings.json"),
        r#"{"lastChangelogVersion":"1.1.0"}"#,
    );
    home.seed(&profile.join("extensions/planted/index.ts"), "evil");
    home.seed(&profile.join("trust.json"), "{}");
    home.seed(&profile.join("index/search.db"), "index");

    home.prepare();

    assert_eq!(std::fs::read_to_string(&conversation).unwrap(), "turns");
    assert_eq!(home.settings()["theme"], "dark");
    assert!(!profile.join("extensions/planted").exists());
    assert!(!profile.join("trust.json").exists());
    assert_eq!(
        std::fs::read_to_string(profile.join("index/search.db")).unwrap(),
        "index"
    );
}

#[test]
fn building_the_profile_never_touches_the_users_pi_dir() {
    let home = Home::new();
    let before = snapshot(&home.path().join(".pi"));

    home.prepare();
    home.prepare();

    assert_eq!(snapshot(&home.path().join(".pi")), before);
}
