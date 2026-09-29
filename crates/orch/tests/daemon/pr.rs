use std::time::Duration;

use orch_core::SessionId;
use orch_protocol::{
    AgentStateView, CreateSession, Landing, PhaseView, PrChecksView, PrReviewView, PrView, Reply,
    Request, RequestError,
};

use crate::common::*;

async fn open_pr(env: &Env, client: &mut TestClient) -> (SessionId, PaneView) {
    let repo = env.repo("app");
    env.remote(&repo);
    env.fake_gh();
    let (id, pane) = idle_session(env, client, "Add caching").await;
    let view = client.sessions[&id].clone();
    commit(&view.worktree, "cache.rs", "cache\n");
    std::fs::write(view.worktree.join("uncommitted.rs"), "wip\n").unwrap();
    let opened = client
        .request(Request::Land {
            session: id.clone(),
            landing: Landing::Pr {
                title: "Add caching".into(),
                body: "Caches things.".into(),
            },
        })
        .await;
    assert_eq!(
        opened,
        Ok(Reply::PrOpened {
            number: 42,
            committed_changes: true,
        })
    );
    (id, pane)
}

async fn refresh(client: &mut TestClient, id: &SessionId) -> Result<Reply, RequestError> {
    client
        .request(Request::RefreshPr {
            session: id.clone(),
        })
        .await
}

async fn pr_becomes(client: &mut TestClient, id: &SessionId, expected: PrView) {
    client
        .until(id, &format!("{expected:?}"), |view| {
            view.flags.pr.as_ref() == Some(&expected)
        })
        .await;
}

#[tokio::test]
async fn a_pr_landing_pushes_everything_and_opens_a_pr_while_the_session_stays_alive() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = open_pr(&env, &mut client).await;
    let view = client
        .until(&id, "PR open", |view| view.phase == PhaseView::PrOpen)
        .await;

    assert_eq!(view.flags.pr_number, Some(42));
    assert!(view.holder_pid.is_some());
    assert!(view.worktree.exists());
    let remote = env.path("remote.git");
    let branch = &view.branch;
    assert_eq!(
        git(&remote, &["show", &format!("{branch}:cache.rs")]),
        "cache"
    );
    assert_eq!(
        git(&remote, &["show", &format!("{branch}:uncommitted.rs")]),
        "wip"
    );
    assert_eq!(
        git(&view.worktree, &["log", "-1", "--format=%s"]),
        "Add caching"
    );
    let calls = env.gh_calls();
    let expected = format!(
        "{}\npr\ncreate\n--head\n{branch}\n--base\nmain\n--title\nAdd caching\n--body\nCaches things.\n",
        env.path("repos/app").display()
    );
    assert!(calls.starts_with(&expected), "{calls}");
}

#[tokio::test]
async fn pr_checks_reviews_and_new_comments_are_followed_until_the_merge_cleans_up() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = open_pr(&env, &mut client).await;
    let view = client.sessions[&id].clone();
    let holder = client.holder_pid(&id).unwrap();

    env.gh_reports(
        r#"{"state":"OPEN","reviewDecision":"CHANGES_REQUESTED",
            "statusCheckRollup":[{"status":"COMPLETED","conclusion":"SUCCESS"},{"status":"IN_PROGRESS","conclusion":""}],
            "comments":[{"body":"a"},{"body":"b"}]}"#,
    );
    assert_eq!(refresh(&mut client, &id).await, Ok(Reply::Done));
    pr_becomes(
        &mut client,
        &id,
        PrView {
            checks: PrChecksView::Pending,
            review: PrReviewView::ChangesRequested,
            new_comments: 2,
            closed: false,
        },
    )
    .await;

    client
        .request(Request::View {
            session: Some(id.clone()),
            focused: true,
        })
        .await
        .unwrap();
    client
        .request(Request::View {
            session: None,
            focused: true,
        })
        .await
        .unwrap();
    env.gh_reports(
        r#"{"state":"OPEN","reviewDecision":"APPROVED",
            "statusCheckRollup":[{"status":"COMPLETED","conclusion":"FAILURE"},{"state":"SUCCESS"}],
            "comments":[{"body":"a"},{"body":"b"},{"body":"c"}]}"#,
    );
    refresh(&mut client, &id).await.unwrap();
    pr_becomes(
        &mut client,
        &id,
        PrView {
            checks: PrChecksView::Failing,
            review: PrReviewView::Approved,
            new_comments: 1,
            closed: false,
        },
    )
    .await;

    env.gh_reports(
        r#"{"state":"MERGED","reviewDecision":"APPROVED","statusCheckRollup":[],"comments":[]}"#,
    );
    assert_eq!(refresh(&mut client, &id).await, Ok(Reply::Done));
    let landed = client
        .until(&id, "Landed and cleaned up", |view| {
            view.phase == PhaseView::Landed && view.port_base.is_none()
        })
        .await;
    assert_eq!(landed.holder_pid, None);
    assert!(!view.worktree.exists());
    assert_eq!(
        git(&env.path("repos/app"), &["branch", "--list", &view.branch]),
        ""
    );
    wait_until("the Holder to exit", || process_gone(holder)).await;
}

#[tokio::test]
async fn a_pr_closed_without_merging_is_flagged_and_nothing_is_deleted() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = open_pr(&env, &mut client).await;

    env.gh_reports(
        r#"{"state":"CLOSED","reviewDecision":null,"statusCheckRollup":null,"comments":[]}"#,
    );
    refresh(&mut client, &id).await.unwrap();

    let closed = client
        .until(&id, "closed", |view| {
            view.flags.pr.as_ref().is_some_and(|pr| pr.closed)
        })
        .await;
    assert_eq!(closed.phase, PhaseView::PrOpen);
    assert!(closed.worktree.exists());
    assert!(closed.holder_pid.is_some());
    assert_eq!(closed.port_base, Some(20000));
}

#[tokio::test]
async fn the_daemon_polls_open_prs_and_cleans_up_after_a_merge() {
    let env = Env::new();
    let _daemon = env
        .start_daemon_args(Duration::from_secs(600), &["--pr-poll-ms", "100"])
        .await;
    let mut client = env.client().await;
    let (id, _pane) = open_pr(&env, &mut client).await;

    env.gh_reports(
        r#"{"state":"MERGED","reviewDecision":"","statusCheckRollup":[],"comments":[]}"#,
    );

    let landed = client
        .until(&id, "Landed and cleaned up", |view| {
            view.phase == PhaseView::Landed && view.port_base.is_none()
        })
        .await;
    assert!(!landed.worktree.exists());
}

async fn land_pr(
    client: &mut TestClient,
    id: &SessionId,
    title: &str,
) -> Result<Reply, RequestError> {
    client
        .request(Request::Land {
            session: id.clone(),
            landing: Landing::Pr {
                title: title.into(),
                body: String::new(),
            },
        })
        .await
}

async fn stacked_on(
    env: &Env,
    client: &mut TestClient,
    lower: &SessionId,
    prompt: &str,
) -> (SessionId, PaneView) {
    let mut create = CreateSession::new(env.path("repos/app"), prompt);
    create.base = Some(client.sessions[lower].branch.clone());
    let upper = client.create(create).await;
    client
        .until(&upper, "running", |view| view.agent.is_some())
        .await;
    let mut pane = env.pane(&upper, PANE).await;
    pane.hook(&hook("SessionStart", r#""source":"startup""#))
        .await;
    client
        .until(&upper, "Idle", |view| {
            view.agent == Some(AgentStateView::Idle)
        })
        .await;
    (upper, pane)
}

#[tokio::test]
async fn a_stacked_pr_pushes_its_orchestrator_base_branch_first() {
    let env = Env::new();
    let repo = env.repo("app");
    let remote = env.remote(&repo);
    env.fake_gh();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (lower, _lower_pane) = idle_session(&env, &mut client, "Lower").await;
    let lower_view = client.sessions[&lower].clone();
    commit(&lower_view.worktree, "lower.txt", "lower\n");
    let (upper, _pane) = stacked_on(&env, &mut client, &lower, "Upper").await;
    commit(&client.sessions[&upper].worktree, "upper.txt", "upper\n");

    let opened = land_pr(&mut client, &upper, "Upper").await;

    assert!(
        matches!(opened, Ok(Reply::PrOpened { number: 42, .. })),
        "{opened:?}"
    );
    assert_eq!(
        git(
            &remote,
            &["show", &format!("{}:lower.txt", lower_view.branch)]
        ),
        "lower"
    );
    assert!(
        env.gh_calls()
            .contains(&format!("--base\n{}\n", lower_view.branch))
    );
}

#[tokio::test]
async fn a_pr_onto_a_base_missing_on_origin_is_refused_clearly() {
    let env = Env::new();
    let repo = env.repo("app");
    env.remote(&repo);
    env.fake_gh();
    git(&repo, &["branch", "feature"]);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let mut create = CreateSession::new(&repo, "On feature");
    create.base = Some("feature".into());
    let id = client.create(create).await;
    client
        .until(&id, "running", |view| view.agent.is_some())
        .await;
    let mut pane = env.pane(&id, PANE).await;
    pane.hook(&hook("SessionStart", "")).await;
    client
        .until(&id, "Idle", |view| view.agent == Some(AgentStateView::Idle))
        .await;
    commit(&client.sessions[&id].worktree, "f.txt", "f\n");

    let refused = land_pr(&mut client, &id, "On feature").await;

    let Err(RequestError::Refused { message }) = refused else {
        panic!("expected a refusal: {refused:?}");
    };
    assert!(message.contains("feature is not on origin"), "{message}");
    assert!(!env.gh_calls().contains("create"));
}

#[tokio::test]
async fn landing_the_base_of_a_stacked_pr_retargets_that_pr_on_github() {
    let env = Env::new();
    let repo = env.repo("app");
    env.remote(&repo);
    env.fake_gh();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (lower, _lower_pane) = idle_session(&env, &mut client, "Lower").await;
    commit(&client.sessions[&lower].worktree, "lower.txt", "lower\n");
    let (upper, _pane) = stacked_on(&env, &mut client, &lower, "Upper").await;
    commit(&client.sessions[&upper].worktree, "upper.txt", "upper\n");
    land_pr(&mut client, &upper, "Upper").await.unwrap();

    let landed = client
        .request(Request::Land {
            session: lower.clone(),
            landing: Landing::Squash {
                message: "Lower".into(),
            },
        })
        .await;

    assert!(
        matches!(landed, Ok(Reply::Landed { warning: None, .. })),
        "{landed:?}"
    );
    assert!(
        env.gh_calls().contains("pr\nedit\n42\n--base\nmain\n"),
        "{}",
        env.gh_calls()
    );
    let view = client
        .until(&upper, "retargeted", |view| view.base == "main")
        .await;
    assert!(view.flags.needs_rebase);
}

#[tokio::test]
async fn a_closed_pr_can_be_reopened_on_github_or_abandoned_for_a_squash() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = open_pr(&env, &mut client).await;
    let closed = r#"{"state":"CLOSED","reviewDecision":null,"statusCheckRollup":null,"comments":[],"reviews":[]}"#;

    let abandon = Request::AbandonPr {
        session: id.clone(),
    };
    assert!(matches!(
        client.request(abandon.clone()).await,
        Err(RequestError::Refused { .. })
    ));

    env.gh_reports(closed);
    refresh(&mut client, &id).await.unwrap();
    client
        .until(&id, "closed", |view| {
            view.flags.pr.as_ref().is_some_and(|pr| pr.closed)
        })
        .await;
    env.gh_reports(r#"{"state":"OPEN","reviewDecision":null,"statusCheckRollup":null,"comments":[],"reviews":[]}"#);
    refresh(&mut client, &id).await.unwrap();
    client
        .until(&id, "reopened", |view| {
            view.flags.pr.as_ref().is_some_and(|pr| !pr.closed)
        })
        .await;

    env.gh_reports(closed);
    refresh(&mut client, &id).await.unwrap();
    client
        .until(&id, "closed again", |view| {
            view.flags.pr.as_ref().is_some_and(|pr| pr.closed)
        })
        .await;
    assert_eq!(client.request(abandon).await, Ok(Reply::Done));
    let active = client
        .until(&id, "Active", |view| view.phase == PhaseView::Active)
        .await;
    assert_eq!(active.flags.pr, None);
    std::fs::write(active.worktree.join("more.rs"), "more\n").unwrap();
    let landed = client
        .request(Request::Land {
            session: id.clone(),
            landing: Landing::Squash {
                message: "Add caching".into(),
            },
        })
        .await;
    assert!(matches!(landed, Ok(Reply::Landed { .. })), "{landed:?}");
}

#[tokio::test]
async fn review_comments_count_as_new_comments() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = open_pr(&env, &mut client).await;

    env.gh_reports(
        r#"{"state":"OPEN","reviewDecision":"REVIEW_REQUIRED","statusCheckRollup":[],
            "comments":[{"body":"a"}],
            "reviews":[{"body":"please rename","state":"COMMENTED"},{"body":"","state":"APPROVED"}]}"#,
    );
    refresh(&mut client, &id).await.unwrap();

    pr_becomes(
        &mut client,
        &id,
        PrView {
            checks: PrChecksView::None,
            review: PrReviewView::ReviewRequired,
            new_comments: 2,
            closed: false,
        },
    )
    .await;
}

#[tokio::test]
async fn work_committed_for_a_pr_stays_and_is_named_when_opening_the_pr_fails() {
    let env = Env::new();
    let repo = env.repo("app");
    env.remote(&repo);
    env.fake_gh();
    env.gh_switch("fail-create", true);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = idle_session(&env, &mut client, "Half done").await;
    let worktree = client.sessions[&id].worktree.clone();
    std::fs::write(worktree.join("wip.rs"), "wip\n").unwrap();

    let failed = land_pr(&mut client, &id, "Half done").await;

    let Err(RequestError::Refused { message }) = failed else {
        panic!("expected a refusal: {failed:?}");
    };
    assert!(message.contains("committed"), "{message}");
    assert!(message.contains("Half done"), "{message}");
    assert!(message.contains("permission denied"), "{message}");
    assert_eq!(git(&worktree, &["log", "-1", "--format=%s"]), "Half done");
    assert_eq!(git(&worktree, &["status", "--porcelain"]), "");
    settled(&mut client).await;
    assert_eq!(client.sessions[&id].phase, PhaseView::Active);
}

#[tokio::test]
async fn a_suspended_pr_session_keeps_the_daemon_alive_and_is_still_polled() {
    let env = Env::new();
    let mut daemon = env
        .start_daemon_args(Duration::from_millis(300), &["--pr-poll-ms", "500"])
        .await;
    let mut client = env.client().await;
    let (id, _pane) = open_pr(&env, &mut client).await;
    kill(client.holder_pid(&id).unwrap(), "-KILL");
    client
        .until(&id, "Suspended", |view| view.phase == PhaseView::Suspended)
        .await;
    drop(_pane);
    drop(client);

    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert!(!daemon.has_exited());

    env.gh_reports(r#"{"state":"MERGED","reviewDecision":"","statusCheckRollup":[],"comments":[],"reviews":[]}"#);
    let mut client = env.client().await;
    let landed = client
        .until(&id, "Landed and cleaned up", |view| {
            view.phase == PhaseView::Landed && view.port_base.is_none()
        })
        .await;
    assert!(!landed.worktree.exists());
}

#[tokio::test]
async fn a_restarted_daemon_polls_open_prs_right_away() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = open_pr(&env, &mut client).await;
    daemon.kill();

    env.gh_reports(r#"{"state":"MERGED","reviewDecision":"","statusCheckRollup":[],"comments":[],"reviews":[]}"#);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    client
        .until(&id, "Landed and cleaned up", |view| {
            view.phase == PhaseView::Landed && view.port_base.is_none()
        })
        .await;
}

#[tokio::test]
async fn typing_into_a_session_is_refused_while_it_is_being_landed() {
    let env = Env::new();
    let repo = env.repo("app");
    env.remote(&repo);
    env.fake_gh();
    env.gh_switch("slow", true);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut client, "Slow PR").await;
    std::fs::write(client.sessions[&id].worktree.join("x.rs"), "x\n").unwrap();

    let mut lander = env.client().await;
    let session = id.clone();
    let landing = tokio::spawn(async move { land_pr(&mut lander, &session, "Slow PR").await });
    wait_until("gh pr create", || env.gh_calls().contains("create")).await;
    pane.type_line("print typed-mid-landing").await;
    pane.wait_for("input dropped", |pane| !pane.dropped.is_empty())
        .await;

    assert!(matches!(landing.await.unwrap(), Ok(Reply::PrOpened { .. })));
    pane.type_line("print typed-after-landing").await;
    pane.wait_for_text("typed-after-landing").await;
    assert!(!pane.text().contains("typed-mid-landing"));
}
