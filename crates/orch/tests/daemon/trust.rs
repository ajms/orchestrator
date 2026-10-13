use std::path::Path;

use orch_core::SessionId;
use orch_protocol::{CreateSession, Landing, PhaseView, Reply, Request, RequestError};

use crate::common::*;

const AGENT: &str = "[agent]\nargs = [\"fake-agent\", \"--\"]\n";

async fn untrusted(
    client: &mut TestClient,
    request: Request,
) -> (std::path::PathBuf, String, Vec<String>) {
    match client.request(request).await {
        Err(RequestError::Untrusted { repo, hash, items }) => (repo, hash, items),
        other => panic!("expected an Untrusted refusal, got {other:?}"),
    }
}

async fn approve(client: &mut TestClient, repo: &Path, hash: String) {
    let approved = client
        .request(Request::ApproveTrust {
            repo: repo.to_path_buf(),
            hash,
        })
        .await;
    assert_eq!(approved, Ok(Reply::Done));
}

async fn trusted_repo(env: &Env, client: &mut TestClient, file: &str) -> std::path::PathBuf {
    let repo = env.repo("app");
    commit(&repo, ".orchestrator.toml", file);
    let refused = Request::CreateSession(CreateSession::new(&repo, "probe"));
    let (_, hash, _) = untrusted(client, refused).await;
    approve(client, &repo, hash).await;
    repo
}

fn change_repo_file(repo: &Path, file: &str) {
    std::fs::write(repo.join(".orchestrator.toml"), file).unwrap();
}

fn discard(session: &SessionId, skip_teardown: bool) -> Request {
    Request::Discard {
        session: session.clone(),
        skip_teardown,
    }
}

#[tokio::test]
async fn an_untrusted_teardown_refuses_discarding_until_it_is_approved() {
    let env = Env::new();
    let marker = env.path("teardown-ran");
    let repo = env.repo("app");
    commit(
        &repo,
        ".orchestrator.toml",
        &format!("teardown = \"touch {}\"\n", marker.display()),
    );
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Untrusted").await;

    let (refused_repo, hash, items) = untrusted(&mut client, discard(&id, false)).await;
    assert_eq!(refused_repo, repo);
    assert_eq!(
        items,
        [format!("Teardown script: touch {}", marker.display())]
    );
    assert_eq!(client.sessions[&id].phase, PhaseView::Active);

    approve(&mut client, &repo, hash).await;
    assert_eq!(client.request(discard(&id, false)).await, Ok(Reply::Done));
    client
        .until(&id, "Discarded", |view| view.phase == PhaseView::Discarded)
        .await;
    assert!(marker.exists());
}

#[tokio::test]
async fn an_untrusted_teardown_can_be_skipped_explicitly() {
    let env = Env::new();
    let marker = env.path("teardown-ran");
    let repo = env.repo("app");
    commit(
        &repo,
        ".orchestrator.toml",
        &format!("teardown = \"touch {}\"\n", marker.display()),
    );
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Skip it").await;

    assert_eq!(client.request(discard(&id, true)).await, Ok(Reply::Done));
    let view = client
        .until(&id, "Discarded and cleaned up", |view| {
            view.phase == PhaseView::Discarded && view.port_base.is_none()
        })
        .await;
    let error = view.error.unwrap_or_default();
    assert!(error.contains("Teardown script was skipped"), "{error}");
    assert!(!marker.exists());
}

#[tokio::test]
async fn a_squash_landing_with_an_untrusted_teardown_is_refused_before_anything_lands() {
    let env = Env::new();
    let repo = env.repo("app");
    commit(&repo, ".orchestrator.toml", "teardown = \"true\"\n");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = idle_session(&env, &mut client, "Land it").await;
    commit(&client.sessions[&id].worktree, "landed.txt", "work\n");
    let main_before = git(&repo, &["rev-parse", "main"]);

    let land = Request::Land {
        session: id.clone(),
        landing: Landing::Squash {
            message: "Land it".into(),
        },
        skip_teardown: false,
    };
    let (refused_repo, _, items) = untrusted(&mut client, land).await;

    assert_eq!(refused_repo, repo);
    assert_eq!(items, ["Teardown script: true"]);
    assert_eq!(git(&repo, &["rev-parse", "main"]), main_before);
}

#[tokio::test]
async fn a_changed_agent_config_refuses_preset_changes_and_resume_until_approved() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let repo = trusted_repo(&env, &mut client, AGENT).await;
    let (id, mut pane) = idle_session(&env, &mut client, "Agent").await;
    change_repo_file(
        &repo,
        "[agent]\nargs = [\"fake-agent\", \"--\", \"--changed\"]\n",
    );

    let preset = Request::SetPreset {
        session: id.clone(),
        preset: "plan".into(),
    };
    let (refused_repo, _, items) = untrusted(&mut client, preset).await;
    assert_eq!(refused_repo, repo);
    assert!(items[0].contains("--changed"), "{items:?}");

    pane.type_line("exit 0").await;
    client
        .until(&id, "Exited", |view| {
            view.agent == Some(orch_protocol::AgentStateView::Exited)
        })
        .await;
    let resume = Request::Resume {
        session: id.clone(),
    };
    let (_, hash, _) = untrusted(&mut client, resume.clone()).await;
    approve(&mut client, &repo, hash).await;
    assert_eq!(client.request(resume).await, Ok(Reply::Done));
}

#[tokio::test]
async fn a_changed_setup_script_refuses_retry_and_start_anyway_until_approved() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let repo = trusted_repo(&env, &mut client, &format!("setup = \"false\"\n{AGENT}")).await;
    let id = client.create(CreateSession::new(&repo, "Setup")).await;
    client
        .until(&id, "Setup failed", |view| {
            view.phase == PhaseView::SetupFailed
        })
        .await;
    change_repo_file(&repo, &format!("setup = \"true\"\n{AGENT}"));

    let retry = Request::RetrySetup {
        session: id.clone(),
    };
    let (refused_repo, _, items) = untrusted(&mut client, retry).await;
    assert_eq!(refused_repo, repo);
    assert!(
        items.contains(&"Setup script: true".to_string()),
        "{items:?}"
    );
    let start = Request::StartAnyway {
        session: id.clone(),
    };
    let (_, hash, _) = untrusted(&mut client, start.clone()).await;
    assert_eq!(client.sessions[&id].phase, PhaseView::SetupFailed);

    approve(&mut client, &repo, hash).await;
    assert_eq!(client.request(start).await, Ok(Reply::Done));
}
