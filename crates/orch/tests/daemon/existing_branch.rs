use std::path::Path;
use std::time::Duration;

use orch_core::SessionId;
use orch_protocol::{CreateSession, Landing, PhaseView, Reply, Request, RequestError};

use crate::common::*;

fn on_branch(repo: &Path, branch: &str) -> CreateSession {
    let mut create = CreateSession::new(repo, "Continue the feature");
    create.branch = Some(branch.into());
    create
}

fn feature_branch(repo: &Path) -> String {
    git(repo, &["checkout", "-q", "-b", "feature-x"]);
    commit(repo, "feature.txt", "feature\n");
    git(repo, &["checkout", "-q", "main"]);
    git(repo, &["rev-parse", "feature-x"])
}

async fn running_on(client: &mut TestClient, create: CreateSession) -> SessionId {
    let id = client.create(create).await;
    client
        .until(&id, "running", |view| view.agent.is_some())
        .await;
    id
}

async fn idle_on(env: &Env, client: &mut TestClient, create: CreateSession) -> SessionId {
    let id = running_on(client, create).await;
    let mut pane = env.pane(&id, PANE).await;
    pane.hook(&hook("SessionStart", r#""source":"startup""#))
        .await;
    client
        .until(&id, "Idle", |view| {
            view.agent == Some(orch_protocol::AgentStateView::Idle)
        })
        .await;
    id
}

async fn refusal(client: &mut TestClient, create: CreateSession) -> String {
    match client.request(Request::CreateSession(create)).await {
        Err(RequestError::Refused { message }) => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn branch_tip(repo: &Path, branch: &str) -> String {
    git(repo, &["rev-parse", &format!("refs/heads/{branch}")])
}

#[tokio::test]
async fn a_session_continues_an_existing_branch_and_discarding_it_keeps_the_branch() {
    let env = Env::new();
    let repo = env.repo("app");
    let tip = feature_branch(&repo);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    let id = running_on(&mut client, on_branch(&repo, "feature-x")).await;
    let view = client.sessions[&id].clone();
    assert_eq!(view.branch, "feature-x");
    assert_eq!(
        view.worktree,
        repo.join(".orchestrator/worktrees/feature-x")
    );
    assert_eq!(git(&view.worktree, &["rev-parse", "HEAD"]), tip);
    commit(&view.worktree, "more.txt", "more\n");
    let worked = git(&view.worktree, &["rev-parse", "HEAD"]);
    std::fs::write(view.worktree.join("scratch.txt"), "scratch\n").unwrap();

    let preview = client
        .request(Request::DiscardPreview {
            session: id.clone(),
        })
        .await;
    assert_eq!(
        preview,
        Ok(Reply::DiscardPreview {
            uncommitted: vec!["scratch.txt".into()],
            unlanded: Vec::new(),
            branch_kept: true,
        })
    );
    let reply = client
        .request(Request::Discard {
            session: id.clone(),
            skip_teardown: false,
        })
        .await;
    assert_eq!(reply, Ok(Reply::Done));
    client
        .until(&id, "Discarded and cleaned up", |view| {
            view.phase == PhaseView::Discarded && view.port_base.is_none()
        })
        .await;

    assert!(!view.worktree.exists());
    assert_eq!(branch_tip(&repo, "feature-x"), worked);
}

#[tokio::test]
async fn landing_an_existing_branch_as_a_squash_keeps_the_branch() {
    let env = Env::new();
    let repo = env.repo("app");
    feature_branch(&repo);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = idle_on(&env, &mut client, on_branch(&repo, "feature-x")).await;
    let worktree = client.sessions[&id].worktree.clone();
    commit(&worktree, "more.txt", "more\n");
    let worked = git(&worktree, &["rev-parse", "HEAD"]);

    let landed = client
        .request(Request::Land {
            session: id.clone(),
            landing: Landing::Squash {
                message: "Finish the feature".into(),
            },
            skip_teardown: false,
        })
        .await;

    assert!(
        matches!(landed, Ok(Reply::Landed { warning: None, .. })),
        "{landed:?}"
    );
    assert!(!worktree.exists());
    assert_eq!(branch_tip(&repo, "feature-x"), worked);
}

#[tokio::test]
async fn a_merged_pr_of_an_existing_branch_keeps_the_branch() {
    let env = Env::new();
    let repo = env.repo("app");
    env.remote(&repo);
    env.fake_gh();
    feature_branch(&repo);
    let _daemon = env
        .start_daemon_args(Duration::from_secs(600), &["--pr-poll-ms", "100"])
        .await;
    let mut client = env.client().await;
    let id = idle_on(&env, &mut client, on_branch(&repo, "feature-x")).await;
    let worktree = client.sessions[&id].worktree.clone();
    commit(&worktree, "more.txt", "more\n");
    let worked = git(&worktree, &["rev-parse", "HEAD"]);
    let opened = client
        .request(Request::Land {
            session: id.clone(),
            landing: Landing::Pr {
                title: "Finish the feature".into(),
                body: String::new(),
            },
            skip_teardown: false,
        })
        .await;
    assert!(matches!(opened, Ok(Reply::PrOpened { .. })), "{opened:?}");

    env.gh_reports(
        r#"{"state":"MERGED","reviewDecision":"","statusCheckRollup":[],"comments":[]}"#,
    );
    client
        .until(&id, "Landed and cleaned up", |view| {
            view.phase == PhaseView::Landed && view.port_base.is_none()
        })
        .await;

    assert!(!worktree.exists());
    assert_eq!(branch_tip(&repo, "feature-x"), worked);
}

#[tokio::test]
async fn a_session_cannot_start_on_a_base_branch() {
    let env = Env::new();
    let repo = env.repo("app");
    let remote = env.remote(&repo);
    git(&remote, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git(&repo, &["checkout", "-q", "-b", "work"]);
    git(&repo, &["branch", "develop"]);
    git(&repo, &["branch", "release"]);
    let lone = env.repo("lone");
    git(&lone, &["checkout", "-q", "-b", "trunk"]);
    env.write_config(&format!("[repos.{repo:?}]\nbase = \"develop\"\n"));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let mut configured_elsewhere = on_branch(&repo, "develop");
    configured_elsewhere.base = Some("release".into());
    let mut own_base = on_branch(&repo, "release");
    own_base.base = Some("release".into());
    let mut current_elsewhere = on_branch(&lone, "trunk");
    current_elsewhere.base = Some("main".into());

    let origin_default = refusal(&mut client, on_branch(&repo, "main")).await;
    let configured = refusal(&mut client, configured_elsewhere).await;
    let own = refusal(&mut client, own_base).await;
    let current = refusal(&mut client, current_elsewhere).await;

    assert!(
        origin_default.contains("main is the Repo's default Base branch"),
        "{origin_default}"
    );
    assert!(
        configured.contains("develop is the Repo's default Base branch"),
        "{configured}"
    );
    assert!(
        own.contains("release is the Session's Base branch"),
        "{own}"
    );
    assert!(
        current.contains("trunk is the Repo's default Base branch"),
        "{current}"
    );
    assert!(client.session_list().await.is_empty());
}

#[tokio::test]
async fn another_sessions_branch_is_refused_but_its_base_can_be_started_on() {
    let env = Env::new();
    let repo = env.repo("app");
    git(&repo, &["branch", "lower"]);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let mut stacked = CreateSession::new(&repo, "Stacked work");
    stacked.base = Some("lower".into());
    let upper = running_on(&mut client, stacked).await;
    let owned = client.sessions[&upper].branch.clone();

    let refused = refusal(&mut client, on_branch(&repo, &owned)).await;
    let lower = running_on(&mut client, on_branch(&repo, "lower")).await;

    assert!(
        refused.contains(&format!("{owned} is the Branch of Session stacked-work")),
        "{refused}"
    );
    assert_eq!(client.sessions[&lower].branch, "lower");
}

#[tokio::test]
async fn a_branch_only_on_origin_is_fetched_and_the_session_starts_on_it() {
    let env = Env::new();
    let repo = env.repo("app");
    env.remote(&repo);
    let tip = feature_branch(&repo);
    git(&repo, &["push", "-q", "origin", "feature-x"]);
    git(&repo, &["branch", "-D", "feature-x"]);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    let id = running_on(&mut client, on_branch(&repo, "feature-x")).await;

    let view = &client.sessions[&id];
    assert_eq!(view.branch, "feature-x");
    assert_eq!(git(&view.worktree, &["rev-parse", "HEAD"]), tip);
}

#[tokio::test]
async fn a_failed_creation_deletes_a_branch_it_fetched_but_keeps_one_that_was_local() {
    let env = Env::new();
    let repo = env.repo("app");
    env.remote(&repo);
    let tip = feature_branch(&repo);
    git(&repo, &["push", "-q", "origin", "feature-x"]);
    git(&repo, &["branch", "-D", "feature-x"]);
    git(&repo, &["branch", "local-y"]);
    env.write_config("[ports]\nstart = 20000\nend = 20009\nblock_size = 10\n");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    client
        .create(CreateSession::new(&repo, "Holds the ports"))
        .await;

    refusal(&mut client, on_branch(&repo, "feature-x")).await;
    refusal(&mut client, on_branch(&repo, "local-y")).await;

    assert_eq!(git(&repo, &["branch", "--list", "feature-x"]), "");
    assert_eq!(
        git(&repo, &["rev-parse", "refs/remotes/origin/feature-x"]),
        tip
    );
    assert!(!repo.join(".orchestrator/worktrees/feature-x").exists());
    assert_eq!(git(&repo, &["branch", "--list", "local-y"]), "  local-y");
    assert!(!repo.join(".orchestrator/worktrees/local-y").exists());
}

#[tokio::test]
async fn the_branch_of_a_session_whose_worktree_is_gone_is_still_refused() {
    let env = Env::new();
    let repo = env.repo("app");
    env.write_config(&format!("[repos.{repo:?}]\nsetup = \"exit 1\"\n"));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = client.create(CreateSession::new(&repo, "Broken")).await;
    let failed = client
        .until(&id, "Setup failed", |view| {
            view.phase == PhaseView::SetupFailed
        })
        .await;
    std::fs::remove_dir_all(&failed.worktree).unwrap();
    git(&repo, &["worktree", "prune"]);

    let refused = refusal(&mut client, on_branch(&repo, &failed.branch)).await;

    assert!(
        refused.contains(&format!(
            "{} is the Branch of Session broken",
            failed.branch
        )),
        "{refused}"
    );
}
