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
        "branch_prefix = \"me/\"\n[defaults.presets.careful]\nmode = \"default\"\n[repos.{repo:?}]\nreview_command = \"nvim -d\"\n"
    ));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    std::fs::create_dir_all(repo.join("src")).unwrap();
    let resolved = settings(&mut client, &repo.join("src")).await;

    assert_eq!(resolved.repo, repo);
    assert!(resolved.presets.contains(&"careful".to_string()));
    assert!(resolved.presets.contains(&"tight".to_string()));
    assert!(resolved.presets.contains(&"plan".to_string()));
    assert_eq!(resolved.default_preset.as_deref(), Some("tight"));
    assert_eq!(resolved.default_base.as_deref(), Some("develop"));
    assert_eq!(resolved.review_command.as_deref(), Some("nvim -d"));
    assert_eq!(resolved.branch_prefix, "me/");
    assert_eq!(resolved.trust, None);

    std::fs::write(repo.join(".orchestrator.toml"), "").unwrap();
    let changed = settings(&mut client, &repo).await;
    assert_eq!(changed.default_base.as_deref(), Some("main"));
    assert!(!changed.presets.contains(&"tight".to_string()));
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

    assert!(!resolved.presets.contains(&"yolo".to_string()));
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
    assert!(
        settings(&mut client, &repo)
            .await
            .presets
            .contains(&"yolo".to_string())
    );
}

#[tokio::test]
async fn repo_settings_offer_the_built_in_agents_and_say_why_one_is_unavailable() {
    let env = Env::new();
    let repo = env.repo("app");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    let resolved = settings(&mut client, &repo).await;
    assert_eq!(resolved.default_agent, "claude");
    assert_eq!(
        resolved.agents,
        vec![AgentChoice {
            name: "claude".into(),
            unavailable: None,
        }]
    );

    env.write_config_with_agent(
        &format!("[repos.{repo:?}]\nagent = \"antigravity\"\n"),
        "/nonexistent/agent",
    );
    let resolved = settings(&mut client, &repo).await;
    assert_eq!(resolved.default_agent, "antigravity");
    let [claude, antigravity] = &resolved.agents[..] else {
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
    assert_eq!(antigravity.name, "antigravity");
    assert!(
        antigravity
            .unavailable
            .as_deref()
            .is_some_and(|why| why.contains("unknown")),
        "{antigravity:?}"
    );
}
