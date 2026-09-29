use orch_protocol::{CreateSession, PhaseView, Reply, Request, RequestError};

use crate::common::*;

fn failing_setup(env: &Env, repo: &std::path::Path) {
    env.write_config(&format!(
        "[repos.{repo:?}]\nsetup = \"echo missing dependency; test -f ready.txt\"\n"
    ));
}

#[tokio::test]
async fn a_failed_setup_shows_its_output_and_can_be_retried() {
    let env = Env::new();
    let repo = env.repo("app");
    failing_setup(&env, &repo);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = client
        .create(CreateSession::new(&repo, "Add caching"))
        .await;

    let failed = client
        .until(&id, "Setup failed", |view| {
            view.phase == PhaseView::SetupFailed
        })
        .await;
    assert!(failed.setup_output.unwrap().contains("missing dependency"));
    assert_eq!(failed.agent, None);
    assert_eq!(failed.holder_pid, None);

    std::fs::write(failed.worktree.join("ready.txt"), "").unwrap();
    let retried = client
        .request(Request::RetrySetup {
            session: id.clone(),
        })
        .await;
    assert_eq!(retried, Ok(Reply::Done));
    client
        .until(&id, "Active with an Agent", |view| {
            view.phase == PhaseView::Active && view.agent.is_some()
        })
        .await;
}

#[tokio::test]
async fn a_failed_setup_can_be_skipped_to_start_the_agent_anyway() {
    let env = Env::new();
    let repo = env.repo("app");
    failing_setup(&env, &repo);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = client
        .create(CreateSession::new(&repo, "Add caching"))
        .await;
    client
        .until(&id, "Setup failed", |view| {
            view.phase == PhaseView::SetupFailed
        })
        .await;

    let skipped = client
        .request(Request::StartAnyway {
            session: id.clone(),
        })
        .await;
    assert_eq!(skipped, Ok(Reply::Done));
    client
        .until(&id, "Active with an Agent", |view| {
            view.phase == PhaseView::Active && view.agent.is_some()
        })
        .await;
    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text("prompt> Add caching").await;

    let again = client.request(Request::StartAnyway { session: id }).await;
    assert!(
        matches!(again, Err(RequestError::Refused { .. })),
        "{again:?}"
    );
}

#[tokio::test]
async fn a_committed_setup_script_runs_only_once_the_repo_is_trusted() {
    let env = Env::new();
    let repo = env.repo("app");
    commit(&repo, ".orchestrator.toml", "setup = \"touch setup-ran\"\n");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    let refused = client
        .request(Request::CreateSession(CreateSession::new(&repo, "Task")))
        .await;
    let Err(RequestError::Untrusted {
        repo: untrusted,
        hash,
        items,
    }) = refused
    else {
        panic!("expected an Untrusted refusal, got {refused:?}");
    };
    assert_eq!(untrusted, repo);
    assert_eq!(items, ["Setup script: touch setup-ran"]);
    assert!(!repo.join(".orchestrator/worktrees").exists());

    let approved = client
        .request(Request::ApproveTrust {
            repo: repo.clone(),
            hash,
        })
        .await;
    assert_eq!(approved, Ok(Reply::Done));
    let id = client.create(CreateSession::new(&repo, "Task")).await;
    let active = client
        .until(&id, "Active", |view| view.phase == PhaseView::Active)
        .await;
    assert!(active.worktree.join("setup-ran").exists());
}

#[tokio::test]
async fn a_changed_setup_script_lapses_trust_and_refuses_the_retry() {
    let env = Env::new();
    let repo = env.repo("app");
    commit(&repo, ".orchestrator.toml", "setup = \"false\"\n");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let Err(RequestError::Untrusted { hash, .. }) = client
        .request(Request::CreateSession(CreateSession::new(&repo, "Task")))
        .await
    else {
        panic!("expected an Untrusted refusal");
    };
    client
        .request(Request::ApproveTrust {
            repo: repo.clone(),
            hash,
        })
        .await
        .unwrap();
    let id = client.create(CreateSession::new(&repo, "Task")).await;
    client
        .until(&id, "Setup failed", |view| {
            view.phase == PhaseView::SetupFailed
        })
        .await;

    std::fs::write(
        repo.join(".orchestrator.toml"),
        "setup = \"touch sneaky\"\n",
    )
    .unwrap();
    let retried = client
        .request(Request::RetrySetup {
            session: id.clone(),
        })
        .await;
    assert!(
        matches!(&retried, Err(RequestError::Untrusted { items, .. }) if items == &["Setup script: touch sneaky"]),
        "{retried:?}"
    );
    settled(&mut client).await;
    let failed = client.sessions[&id].clone();
    assert_eq!(failed.phase, PhaseView::SetupFailed);
    assert!(!failed.worktree.join("sneaky").exists());
}

#[tokio::test]
async fn setup_output_streams_and_a_daemon_restart_kills_the_script_and_keeps_its_log() {
    let env = Env::new();
    let repo = env.repo("app");
    env.write_config(&format!(
        "[repos.{repo:?}]\nsetup = \"echo installing; sleep 60; echo never\"\n"
    ));
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = client.create(CreateSession::new(&repo, "Slow setup")).await;
    let running = client
        .until(&id, "streamed Setup output", |view| {
            view.setup_output
                .as_deref()
                .is_some_and(|output| output.contains("installing"))
        })
        .await;
    assert_eq!(running.phase, PhaseView::SettingUp);

    daemon.kill();
    let daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let failed = client
        .until(&id, "Setup failed", |view| {
            view.phase == PhaseView::SetupFailed
        })
        .await;
    let output = failed.setup_output.unwrap();
    assert!(output.contains("installing"), "{output}");
    assert!(output.contains("interrupted"), "{output}");
    wait_until("the orphaned Setup script to be killed", || {
        env.processes() == [daemon.pid()]
    })
    .await;
}

#[tokio::test]
async fn approving_trust_with_a_stale_hash_is_refused() {
    let env = Env::new();
    let repo = env.repo("app");
    commit(&repo, ".orchestrator.toml", "setup = \"touch setup-ran\"\n");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let approved = client
        .request(Request::ApproveTrust {
            repo: repo.clone(),
            hash: "0123abcd".into(),
        })
        .await;
    assert!(
        matches!(approved, Err(RequestError::Refused { .. })),
        "{approved:?}"
    );
    let create = client
        .request(Request::CreateSession(CreateSession::new(&repo, "Task")))
        .await;
    assert!(
        matches!(create, Err(RequestError::Untrusted { .. })),
        "{create:?}"
    );
}
