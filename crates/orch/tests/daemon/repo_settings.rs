use orch_protocol::{AgentChoice, Reply, RepoSettings, Request, TrustNeeded};

use crate::common::*;

async fn settings(client: &mut TestClient, repo: &std::path::Path) -> RepoSettings {
    match client
        .request(Request::RepoSettings {
            repo: repo.to_path_buf(),
        })
        .await
    {
        Ok(Reply::RepoSettings(settings)) => settings,
        other => panic!("expected RepoSettings, got {other:?}"),
    }
}

fn claude(settings: &RepoSettings) -> &AgentChoice {
    let found = settings.agents.iter().find(|agent| agent.name == "claude");
    found.expect("Claude is offered")
}

fn offered(settings: &RepoSettings) -> Vec<&str> {
    let presets = claude(settings).presets.iter();
    presets.map(|preset| preset.name.as_str()).collect()
}

#[tokio::test]
async fn repo_settings_are_resolved_from_the_repo_config_at_request_time() {
    let env = Env::new();
    let repo = env.repo("app");
    git(&repo, &["branch", "develop"]);
    commit(
        &repo,
        ".orchestrator.toml",
        "base = \"develop\"\npreset = \"tight\"\n[presets.tight]\nmode = \"plan\"\n",
    );
    env.write_config(&format!(
        "branch_prefix = \"me/\"\n[defaults.presets.careful]\nmode = \"default\"\n[defaults.presets.agy]\nmode = \"plan\"\n[defaults.presets.agy.antigravity]\ndeny = [\"command(rm)\"]\n[repos.{repo:?}]\nreview_command = \"nvim -d\"\n"
    ));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    std::fs::create_dir_all(repo.join("src")).unwrap();
    let resolved = settings(&mut client, &repo.join("src")).await;

    assert_eq!(resolved.repo, repo);
    assert!(offered(&resolved).contains(&"careful"));
    assert!(offered(&resolved).contains(&"tight"));
    assert!(offered(&resolved).contains(&"plan"));
    let marked = claude(&resolved)
        .presets
        .iter()
        .filter(|preset| preset.lacks_rules);
    assert_eq!(
        marked
            .map(|preset| preset.name.as_str())
            .collect::<Vec<_>>(),
        ["agy"]
    );
    let default = claude(&resolved).default_preset.as_ref().unwrap();
    assert_eq!(
        (default.name.as_str(), default.unsupported.as_deref()),
        ("tight", None)
    );
    assert_eq!(resolved.default_base.as_deref(), Some("develop"));
    assert_eq!(resolved.review_command.as_deref(), Some("nvim -d"));
    assert_eq!(resolved.branch_prefix, "me/");
    assert_eq!(resolved.trust, None);

    std::fs::write(repo.join(".orchestrator.toml"), "").unwrap();
    let changed = settings(&mut client, &repo).await;
    assert_eq!(changed.default_base.as_deref(), Some("main"));
    assert!(!offered(&changed).contains(&"tight"));
}

#[tokio::test]
async fn an_untrusted_loosening_preset_is_withheld_and_its_trust_request_reported() {
    let env = Env::new();
    let repo = env.repo("app");
    commit(
        &repo,
        ".orchestrator.toml",
        "[presets.yolo]\nmode = \"acceptEdits\"\n",
    );
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    let resolved = settings(&mut client, &repo).await;

    assert!(!offered(&resolved).contains(&"yolo"));
    let Some(TrustNeeded { hash, items }) = resolved.trust else {
        panic!("the Trust request is reported");
    };
    assert_eq!(items, ["Preset yolo"]);
    client
        .request(Request::ApproveTrust {
            repo: repo.clone(),
            hash,
        })
        .await
        .unwrap();
    assert!(offered(&settings(&mut client, &repo).await).contains(&"yolo"));
}

#[tokio::test]
async fn an_agent_that_cannot_express_the_default_preset_gets_the_fallback_and_why() {
    let env = Env::new();
    let repo = env.repo("app");
    commit(
        &repo,
        ".orchestrator.toml",
        "agent = \"nonesuch\"\npreset = \"auto\"\n",
    );
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    let resolved = settings(&mut client, &repo).await;

    let default = |agent: &AgentChoice| {
        let default = agent.default_preset.clone().unwrap();
        (default.name, default.unsupported)
    };
    let nonesuch = resolved
        .agents
        .iter()
        .find(|agent| agent.name == "nonesuch");
    let nonesuch = nonesuch.unwrap();
    assert_eq!(default(nonesuch), ("inherit".into(), Some("auto".into())));
    let offered = nonesuch.presets.iter().map(|preset| preset.name.as_str());
    assert_eq!(offered.collect::<Vec<_>>(), ["inherit"]);
    assert_eq!(default(claude(&resolved)), ("auto".into(), None));
}

#[tokio::test]
async fn repo_settings_offer_the_built_in_agents_and_say_why_one_is_unavailable() {
    let env = Env::new();
    let repo = env.repo("app");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    let resolved = settings(&mut client, &repo).await;
    assert_eq!(resolved.default_agent, "claude");
    let names = resolved.agents.iter().map(|agent| agent.name.as_str());
    assert_eq!(names.collect::<Vec<_>>(), ["claude", "antigravity"]);
    assert_eq!(claude(&resolved).unavailable, None);

    env.write_config_with_agent(
        &format!("[repos.{repo:?}]\nagent = \"nonesuch\"\n"),
        "/nonexistent/agent",
    );
    let resolved = settings(&mut client, &repo).await;
    assert_eq!(resolved.default_agent, "nonesuch");
    let [claude, _, nonesuch] = &resolved.agents[..] else {
        panic!("{:?}", resolved.agents);
    };
    assert_eq!(claude.name, "claude");
    assert!(
        claude
            .unavailable
            .as_deref()
            .is_some_and(|why| why.contains("/nonexistent/agent")),
        "{claude:?}"
    );
    assert_eq!(nonesuch.name, "nonesuch");
    assert!(
        nonesuch
            .unavailable
            .as_deref()
            .is_some_and(|why| why.contains("unknown")),
        "{nonesuch:?}"
    );
}
