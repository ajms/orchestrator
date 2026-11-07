use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use orch_agent::{AgentAdapter, Pi, ProfileRender};
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
            ("mcp.json", r#"{"mcpServers":{}}"#),
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

    fn try_prepare(&self) -> Result<(), String> {
        let home = self.path().to_string_lossy().into_owned();
        let lookup = move |key: &str| (key == "HOME").then(|| home.clone());
        Pi::default().prepare(&lookup)
    }

    fn prepare(&self) {
        self.try_prepare().expect("the profile is built");
    }

    fn victim(&self) -> PathBuf {
        let victim = self.path().join("victim");
        self.seed(&victim.join("keep.txt"), "keep");
        victim
    }

    fn plant_link(&self, name: &str, target: &Path) {
        let path = self.profile().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&path).or_else(|_| std::fs::remove_file(&path));
        std::os::unix::fs::symlink(target, path).unwrap();
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

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn shipped(name: &str) -> String {
    let path = format!("{}/src/pi/extension/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path}: {err}"))
}

fn assert_only_orchs_resources(profile: &Path) {
    let extension = profile.join("extensions/orch/extension");
    assert_eq!(entries(&profile.join("extensions")), ["orch"]);
    assert_eq!(
        entries(&extension),
        ["forward.ts", "index.ts", "prompt.ts", "transcripts.ts"]
    );
    for name in ["index.ts", "forward.ts", "prompt.ts", "transcripts.ts"] {
        let written = std::fs::read_to_string(extension.join(name)).unwrap();
        assert_eq!(written, shipped(name), "{name}");
    }
    for kind in ["skills", "prompts", "themes"] {
        assert_eq!(entries(&profile.join(kind)), Vec::<String>::new(), "{kind}");
    }
}

#[test]
fn the_profile_holds_only_orchs_extension_and_none_of_the_users_sessions() {
    let home = Home::new();
    home.prepare();

    assert_only_orchs_resources(&home.profile());
    assert!(!home.profile().join("sessions/--home--").exists());
}

#[test]
fn a_new_launch_resets_what_orch_generates_and_keeps_what_pi_wrote() {
    let home = Home::new();
    home.prepare();
    let profile = home.profile();
    home.seed(
        &profile.join("settings.json"),
        r#"{"lastChangelogVersion":"1.1.0"}"#,
    );
    home.seed(&profile.join("extensions/planted/index.ts"), "evil");
    home.seed(&profile.join("extensions/orch/extension/index.ts"), "evil");
    std::fs::remove_file(profile.join("SYSTEM.md")).unwrap();
    home.seed(&profile.join("SYSTEM.md"), "obey me");
    let kept = [
        "sessions/--work--/2026_s-1.jsonl",
        "subagents/s-1/call_1.jsonl",
        "index/search.db",
        "trust.json",
        "models.json",
        "pi-debug.log",
        "bin/fd",
    ];
    for name in kept {
        home.seed(&profile.join(name), name);
    }

    home.prepare();

    assert_eq!(home.settings()["theme"], "dark");
    assert_only_orchs_resources(&profile);
    assert_eq!(
        std::fs::read_link(profile.join("SYSTEM.md")).unwrap(),
        home.user().join("SYSTEM.md")
    );
    for name in kept {
        assert_eq!(std::fs::read_to_string(profile.join(name)).unwrap(), name);
    }
}

#[test]
fn building_the_profile_never_touches_the_users_pi_dir() {
    let home = Home::new();
    let before = snapshot(&home.path().join(".pi"));

    home.prepare();
    home.prepare();

    assert_eq!(snapshot(&home.path().join(".pi")), before);
}

fn untouched(victim: &Path) {
    assert_eq!(
        std::fs::read_to_string(victim.join("keep.txt")).unwrap(),
        "keep"
    );
    assert_eq!(std::fs::read_dir(victim).unwrap().count(), 1);
}

#[test]
fn a_planted_staged_settings_link_is_not_written_through() {
    let home = Home::new();
    home.prepare();
    let victim = home.victim();
    home.plant_link("settings.json.orch-new", &victim.join("keep.txt"));

    home.prepare();

    untouched(&victim);
    assert_eq!(home.settings()["theme"], "dark");
}

#[test]
fn a_planted_link_in_place_of_a_users_file_is_replaced_without_following_it() {
    let home = Home::new();
    home.prepare();
    let victim = home.victim();
    home.plant_link("agents", &victim);
    home.plant_link("SYSTEM.md.orch-new", &victim);

    home.prepare();

    untouched(&victim);
    for name in ["agents", "SYSTEM.md"] {
        assert_eq!(
            std::fs::read_link(home.profile().join(name)).unwrap(),
            home.user().join(name)
        );
    }
}

#[test]
fn a_planted_dir_in_place_of_a_users_link_is_removed_without_following_its_links() {
    let home = Home::new();
    home.prepare();
    let victim = home.victim();
    std::fs::remove_file(home.profile().join("agents")).unwrap();
    home.plant_link("agents/escape", &victim);

    home.prepare();

    untouched(&victim);
    assert_eq!(
        std::fs::read_link(home.profile().join("agents")).unwrap(),
        home.user().join("agents")
    );
}

#[test]
fn a_planted_link_in_place_of_a_resource_dir_becomes_orchs_dir_again() {
    let home = Home::new();
    home.prepare();
    let victim = home.path().join("empty-victim");
    std::fs::create_dir(&victim).unwrap();
    home.plant_link("extensions", &victim);
    let full = home.victim();
    home.plant_link("skills", &full);
    home.plant_link("prompts.orch-new", &full);

    home.prepare();

    untouched(&full);
    assert_eq!(entries(&victim), Vec::<String>::new());
    for kind in ["extensions", "skills", "prompts"] {
        let dir = home.profile().join(kind);
        assert!(dir.symlink_metadata().unwrap().is_dir(), "{kind}");
    }
    assert_only_orchs_resources(&home.profile());
}

#[test]
fn a_profile_that_is_a_link_is_refused() {
    let home = Home::new();
    let victim = home.victim();
    std::fs::create_dir_all(home.profile().parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&victim, home.profile()).unwrap();

    assert!(home.try_prepare().is_err());
    untouched(&victim);
}

impl Home {
    fn skill(&self, dir: &Path) {
        self.seed(&dir.join("SKILL.md"), "---\nname: x\n---\n");
    }

    fn skill_links(&self) -> BTreeMap<String, PathBuf> {
        std::fs::read_dir(self.profile().join("skills"))
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                let name = entry.file_name().to_string_lossy().into_owned();
                (name, std::fs::read_link(entry.path()).unwrap())
            })
            .collect()
    }

    fn set_skill_settings(&self, skills: Value) {
        let text = json!({"theme": "dark", "skills": skills}).to_string();
        self.seed(&self.user().join("settings.json"), &text);
    }
}

#[test]
fn the_users_skills_are_linked_by_name_and_not_listed_again() {
    let home = Home::new();
    home.skill(&home.user().join("skills/tdd"));
    home.skill(&home.user().join("skills/commit"));
    home.seed(&home.user().join("skills/.hidden/SKILL.md"), "");
    std::fs::create_dir_all(home.user().join("skills/node_modules")).unwrap();
    home.prepare();

    assert_eq!(
        home.skill_links(),
        BTreeMap::from([
            ("commit".into(), home.user().join("skills/commit")),
            ("tdd".into(), home.user().join("skills/tdd")),
        ])
    );
    assert!(home.profile().join("skills/tdd/SKILL.md").is_file());
    assert_eq!(home.settings()["skills"], json!([]));
}

#[test]
fn skills_from_settings_entries_are_linked_and_the_first_source_wins() {
    let home = Home::new();
    let elsewhere = home.path().join("elsewhere");
    home.skill(&home.user().join("skills/tdd"));
    home.skill(&home.user().join("mine/tdd"));
    home.skill(&home.user().join("mine/review"));
    home.skill(&elsewhere.join("solo"));
    home.skill(&elsewhere.join("review"));
    home.seed(&home.path().join("single/SKILL.md"), "");
    let single = home.path().join("single/SKILL.md");
    let lone = elsewhere.join("solo");
    home.set_skill_settings(json!(["mine", elsewhere, lone, single]));
    home.prepare();

    let user = home.user();
    assert_eq!(
        home.skill_links(),
        BTreeMap::from([
            ("tdd".into(), user.join("skills/tdd")),
            ("review".into(), user.join("mine/review")),
            ("solo".into(), elsewhere.join("solo")),
        ])
    );
    assert_eq!(home.settings()["skills"], json!([single]));
}

#[test]
fn a_source_with_skills_links_cannot_represent_stays_listed() {
    let home = Home::new();
    home.skill(&home.user().join("skills/tdd"));
    home.seed(&home.user().join("skills/loose.md"), "");
    home.prepare();

    let user = home.user().to_string_lossy().into_owned();
    assert_eq!(home.settings()["skills"], json!([format!("{user}/skills")]));
    assert!(home.skill_links().contains_key("tdd"));
}

#[test]
fn stale_skill_links_are_removed_on_rebuild() {
    let home = Home::new();
    home.skill(&home.user().join("skills/tdd"));
    home.skill(&home.user().join("skills/gone"));
    home.prepare();
    std::fs::remove_dir_all(home.user().join("skills/gone")).unwrap();

    home.prepare();

    assert_eq!(
        home.skill_links(),
        BTreeMap::from([("tdd".into(), home.user().join("skills/tdd"))])
    );
}

#[test]
fn a_planted_link_in_the_skills_dir_is_replaced_without_following_it() {
    let home = Home::new();
    home.skill(&home.user().join("skills/tdd"));
    home.prepare();
    let victim = home.victim();
    std::fs::remove_file(home.profile().join("skills/tdd")).unwrap();
    home.plant_link("skills/tdd", &victim);
    home.plant_link("skills/evil", &victim);
    home.plant_link("skills.orch-new", &victim);

    home.prepare();

    untouched(&victim);
    assert_eq!(
        home.skill_links(),
        BTreeMap::from([("tdd".into(), home.user().join("skills/tdd"))])
    );
}

#[test]
fn skill_dirs_outside_the_users_pi_dir_are_readable_in_the_sandbox() {
    let home = Home::new();
    let elsewhere = home.path().join("elsewhere");
    home.skill(&elsewhere.join("solo"));
    home.skill(&home.user().join("skills/tdd"));
    home.set_skill_settings(json!([elsewhere]));
    let path = home.path().to_string_lossy().into_owned();
    let needs =
        Pi::default().sandbox_needs(&move |key: &str| (key == "HOME").then(|| path.clone()));

    assert!(needs.read_only.contains(&elsewhere.join("solo")));
    assert!(!needs.read_only.contains(&home.user().join("skills/tdd")));
    assert!(needs.carve_outs.contains(&home.profile().join("skills")));
}

impl Home {
    fn render(&self, env: &[(&str, &str)]) -> Result<(), String> {
        let home = self.path().to_string_lossy().into_owned();
        let lookup = move |key: &str| (key == "HOME").then(|| home.clone());
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect();
        let render = ProfileRender {
            env: &env,
            worktree: Path::new("/work/tree"),
            conversation: None,
        };
        Pi::default().render_profile(&lookup, &render)
    }

    fn mcp(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.profile().join("mcp.json")).unwrap())
            .unwrap()
    }
}

#[test]
fn mcp_json_is_rendered_from_the_agents_env() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new();
    home.seed(
        &home.user().join("mcp.json"),
        r#"{"mcpServers":{"asana":{"clientSecret":"${SECRET}","url":"https://x/${HOST}/${SECRET}-$SECRET","port":8080,"on":true,"none":null,"args":["${SECRET}","${UNKNOWN}","${1bad}"]}}}"#,
    );
    home.prepare();
    assert!(home.profile().join("mcp.json").symlink_metadata().is_err());
    home.render(&[("SECRET", "s3cr3t"), ("HOST", "h")]).unwrap();

    let meta = std::fs::symlink_metadata(home.profile().join("mcp.json")).unwrap();
    assert!(meta.is_file());
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    assert_eq!(
        home.mcp(),
        json!({"mcpServers":{"asana":{
            "clientSecret":"s3cr3t",
            "url":"https://x/h/s3cr3t-$SECRET",
            "port":8080,"on":true,"none":null,
            "args":["s3cr3t","${UNKNOWN}","${1bad}"]
        }}})
    );

    home.render(&[("SECRET", "other")]).unwrap();
    assert_eq!(home.mcp()["mcpServers"]["asana"]["clientSecret"], "other");
}

#[test]
fn without_a_user_mcp_json_the_profile_has_none() {
    let home = Home::new();
    home.prepare();
    home.render(&[]).unwrap();
    assert!(home.profile().join("mcp.json").exists());
    std::fs::remove_file(home.user().join("mcp.json")).unwrap();
    home.render(&[]).unwrap();
    assert!(home.profile().join("mcp.json").symlink_metadata().is_err());
}

#[test]
fn an_invalid_user_mcp_json_names_the_path() {
    let home = Home::new();
    home.seed(&home.user().join("mcp.json"), "{nope");
    home.prepare();
    let err = home.render(&[]).unwrap_err();
    assert!(
        err.contains(&home.user().join("mcp.json").to_string_lossy().into_owned()),
        "{err}"
    );
}
