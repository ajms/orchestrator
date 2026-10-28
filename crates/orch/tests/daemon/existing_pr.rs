use std::path::{Path, PathBuf};

use orch_core::SessionId;
use orch_protocol::{Landing, PhaseView, Reply, Request, RequestError};

use crate::common::*;

fn pr_json(number: u64, state: &str, base: &str, head: &str) -> String {
    format!(
        r#"{{"number":{number},"title":"Add the feature","state":"{state}","baseRefName":"{base}","headRefName":"{head}","isCrossRepository":false,"statusCheckRollup":[],"reviewDecision":"","comments":[],"reviews":[]}}"#
    )
}

fn pushed_feature(env: &Env) -> PathBuf {
    let repo = env.repo("app");
    env.github_remote(&repo);
    env.fake_gh();
    feature_branch(&repo);
    git(&repo, &["push", "-q", "origin", "feature-x"]);
    repo
}

fn release_only_on_origin(repo: &Path) -> String {
    git(repo, &["branch", "release"]);
    git(repo, &["push", "-q", "origin", "release"]);
    git(repo, &["branch", "-D", "release"]);
    git(repo, &["rev-parse", "refs/remotes/origin/release"])
}

async fn refresh(client: &mut TestClient, id: &SessionId) {
    let reply = client
        .request(Request::RefreshPr {
            session: id.clone(),
        })
        .await;
    assert_eq!(reply, Ok(Reply::Done));
}

#[tokio::test]
async fn a_session_on_an_open_pr_by_number_takes_its_base_and_starts_in_pr_open() {
    let env = Env::new();
    let repo = pushed_feature(&env);
    let release = release_only_on_origin(&repo);
    env.gh_reports(&pr_json(7, "OPEN", "release", "feature-x"));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    let id = client.create(on_branch(&repo, "#7")).await;
    let view = client
        .until(&id, "PR open", |view| view.phase == PhaseView::PrOpen)
        .await;

    assert_eq!(view.branch, "feature-x");
    assert_eq!(view.base, "release");
    assert_eq!(view.flags.pr_number, Some(7));
    assert_eq!(git(&repo, &["rev-parse", "refs/heads/release"]), release);
}

#[tokio::test]
async fn a_pr_by_number_is_refused_when_missing_merged_from_a_fork_or_without_its_branch() {
    let env = Env::new();
    let repo = pushed_feature(&env);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    env.gh_switch("fail-view", true);
    let missing = refusal(&mut client, on_branch(&repo, "#7")).await;
    env.gh_switch("fail-view", false);
    env.gh_reports(&pr_json(7, "MERGED", "main", "feature-x"));
    let merged = refusal(&mut client, on_branch(&repo, "#7")).await;
    env.gh_reports(&pr_json(7, "OPEN", "main", "feature-x").replace(
        r#""isCrossRepository":false"#,
        r#""isCrossRepository":true"#,
    ));
    let fork = refusal(&mut client, on_branch(&repo, "#7")).await;
    env.gh_reports(&pr_json(7, "CLOSED", "main", "gone-y"));
    let gone = refusal(&mut client, on_branch(&repo, "#7")).await;

    assert!(missing.contains("PR #7 wasn't found"), "{missing}");
    assert!(merged.contains("PR #7 is already merged"), "{merged}");
    assert!(fork.contains("PR #7's Branch is in a fork"), "{fork}");
    assert!(
        gone.contains("the PR's Branch gone-y no longer exists"),
        "{gone}"
    );
    assert!(client.session_list().await.is_empty());
    assert_eq!(git(&repo, &["branch", "--list", "gone-y"]), "");
}

#[tokio::test]
async fn a_closed_pr_by_number_starts_an_active_session_without_a_pr_on_the_usual_base() {
    let env = Env::new();
    let repo = pushed_feature(&env);
    release_only_on_origin(&repo);
    env.gh_reports(&pr_json(7, "CLOSED", "release", "feature-x"));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    let id = running_on(&mut client, on_branch(&repo, "#7")).await;

    let view = &client.sessions[&id];
    assert_eq!(view.branch, "feature-x");
    assert_eq!(view.base, "main");
    assert_eq!(view.phase, PhaseView::Active);
    assert_eq!(view.flags.pr_number, None);
}

#[tokio::test]
async fn a_typed_branch_with_an_open_pr_takes_its_base_and_refuses_another() {
    let env = Env::new();
    let repo = pushed_feature(&env);
    release_only_on_origin(&repo);
    env.gh_lists(&format!("[{}]", pr_json(7, "OPEN", "release", "feature-x")));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let mut onto_main = on_branch(&repo, "feature-x");
    onto_main.base = Some("main".into());

    let refused = refusal(&mut client, onto_main).await;
    let id = client.create(on_branch(&repo, "feature-x")).await;
    let view = client
        .until(&id, "PR open", |view| view.phase == PhaseView::PrOpen)
        .await;

    assert!(
        refused.contains("PR #7 targets release, not main"),
        "{refused}"
    );
    assert_eq!(view.base, "release");
    assert_eq!(view.flags.pr_number, Some(7));
}

#[tokio::test]
async fn a_typed_branch_with_several_open_prs_is_refused_asking_for_a_number() {
    let env = Env::new();
    let repo = pushed_feature(&env);
    env.gh_lists(&format!(
        "[{},{}]",
        pr_json(7, "OPEN", "main", "feature-x"),
        pr_json(9, "OPEN", "release", "feature-x")
    ));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    let refused = refusal(&mut client, on_branch(&repo, "feature-x")).await;

    assert!(
        refused.contains("feature-x has 2 open PRs (#7, #9); type #<number> to pick one"),
        "{refused}"
    );
    assert!(client.session_list().await.is_empty());
}

#[tokio::test]
async fn only_comments_made_after_the_session_started_on_a_pr_count_as_new() {
    let env = Env::new();
    let repo = pushed_feature(&env);
    let with_comments = |count: usize| {
        let comments = vec![r#"{"body":"a"}"#; count].join(",");
        pr_json(7, "OPEN", "main", "feature-x")
            .replace(r#""comments":[]"#, &format!(r#""comments":[{comments}]"#))
    };
    env.gh_reports(&with_comments(2));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = client.create(on_branch(&repo, "#7")).await;
    client
        .until(&id, "PR open", |view| view.phase == PhaseView::PrOpen)
        .await;

    refresh(&mut client, &id).await;
    env.gh_reports(&with_comments(3));
    refresh(&mut client, &id).await;

    client
        .until(&id, "one new comment", |view| {
            view.flags.pr.as_ref().map(|pr| pr.new_comments) == Some(1)
        })
        .await;
}

async fn land(
    client: &mut TestClient,
    id: &SessionId,
    landing: Landing,
) -> Result<Reply, RequestError> {
    client
        .request(Request::Land {
            session: id.clone(),
            landing,
            skip_teardown: false,
        })
        .await
}

fn pr_landing() -> Landing {
    Landing::Pr {
        title: "Address the review".into(),
        body: String::new(),
    }
}

#[tokio::test]
async fn landing_a_session_with_an_open_pr_only_pushes_to_it() {
    let env = Env::new();
    let repo = pushed_feature(&env);
    env.gh_reports(&pr_json(7, "OPEN", "main", "feature-x"));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = idle_on(&env, &mut client, on_branch(&repo, "#7")).await;
    let worktree = client.sessions[&id].worktree.clone();
    commit(&worktree, "fix.rs", "fix\n");
    std::fs::write(worktree.join("wip.rs"), "wip\n").unwrap();

    let squash = land(
        &mut client,
        &id,
        Landing::Squash {
            message: "Squash it".into(),
        },
    )
    .await;
    let pushed = land(&mut client, &id, pr_landing()).await;

    let Err(RequestError::Refused { message }) = squash else {
        panic!("expected a refusal: {squash:?}");
    };
    assert!(message.contains("PR #7 is open"), "{message}");
    assert_eq!(
        pushed,
        Ok(Reply::Pushed {
            number: 7,
            committed_changes: true,
        })
    );
    let remote = env.path("remote.git");
    assert_eq!(git(&remote, &["show", "feature-x:fix.rs"]), "fix");
    assert_eq!(git(&remote, &["show", "feature-x:wip.rs"]), "wip");
    assert!(!env.gh_calls().contains("create"));
    assert_eq!(client.sessions[&id].phase, PhaseView::PrOpen);
}

#[tokio::test]
async fn landing_refuses_to_push_when_origin_has_commits_the_branch_lacks() {
    let env = Env::new();
    let repo = pushed_feature(&env);
    env.gh_reports(&pr_json(7, "OPEN", "main", "feature-x"));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = idle_on(&env, &mut client, on_branch(&repo, "#7")).await;
    let worktree = client.sessions[&id].worktree.clone();
    git(&repo, &["checkout", "-q", "-b", "reviewer", "feature-x"]);
    commit(&repo, "review.rs", "review\n");
    git(&repo, &["push", "-q", "origin", "reviewer:feature-x"]);
    git(&repo, &["checkout", "-q", "main"]);
    let theirs = git(&repo, &["rev-parse", "reviewer"]);
    commit(&worktree, "fix.rs", "fix\n");

    let pushed = land(&mut client, &id, pr_landing()).await;

    let Err(RequestError::Refused { message }) = pushed else {
        panic!("expected a refusal: {pushed:?}");
    };
    assert!(message.contains("pull or rebase first"), "{message}");
    let remote = env.path("remote.git");
    assert_eq!(git(&remote, &["rev-parse", "feature-x"]), theirs);
}

#[tokio::test]
async fn a_pr_retargeted_on_github_moves_the_sessions_base_and_asks_for_a_rebase() {
    let env = Env::new();
    let repo = pushed_feature(&env);
    let release = release_only_on_origin(&repo);
    env.gh_reports(&pr_json(7, "OPEN", "main", "feature-x"));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = client.create(on_branch(&repo, "#7")).await;
    client
        .until(&id, "PR open", |view| view.phase == PhaseView::PrOpen)
        .await;

    env.gh_reports(&pr_json(7, "OPEN", "release", "feature-x"));
    refresh(&mut client, &id).await;

    let view = client
        .until(&id, "following release", |view| view.base == "release")
        .await;
    assert!(view.flags.needs_rebase);
    assert_eq!(git(&repo, &["rev-parse", "refs/heads/release"]), release);
}

#[tokio::test]
async fn landing_follows_the_base_of_a_pr_retargeted_since_the_last_poll() {
    let env = Env::new();
    let repo = pushed_feature(&env);
    release_only_on_origin(&repo);
    env.gh_reports(&pr_json(7, "OPEN", "main", "feature-x"));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = idle_on(&env, &mut client, on_branch(&repo, "#7")).await;

    env.gh_reports(&pr_json(7, "OPEN", "release", "feature-x"));
    let pushed = land(&mut client, &id, pr_landing()).await;

    assert!(
        matches!(pushed, Ok(Reply::Pushed { number: 7, .. })),
        "{pushed:?}"
    );
    let view = client
        .until(&id, "following release", |view| view.base == "release")
        .await;
    assert!(view.flags.needs_rebase);
}

#[tokio::test]
async fn a_failed_pr_lookup_refuses_a_typed_branch_unless_origin_is_not_on_github() {
    let env = Env::new();
    let repo = pushed_feature(&env);
    env.gh_switch("fail-list", true);
    let elsewhere = Env::new();
    let other = elsewhere.repo("app");
    elsewhere.remote(&other);
    elsewhere.fake_gh();
    elsewhere.gh_switch("fail-list", true);
    feature_branch(&other);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let _other_daemon = elsewhere.start_daemon().await;
    let mut other_client = elsewhere.client().await;

    let refused = refusal(&mut client, on_branch(&repo, "feature-x")).await;
    let started = running_on(&mut other_client, on_branch(&other, "feature-x")).await;

    assert!(
        refused.contains("looking up the PRs of feature-x failed"),
        "{refused}"
    );
    assert!(refused.contains("Bad credentials"), "{refused}");
    assert_eq!(other_client.sessions[&started].phase, PhaseView::Active);
    assert!(!elsewhere.gh_calls().contains("list"));
}

#[tokio::test]
async fn landing_an_active_session_whose_branch_has_an_open_pr_only_pushes_to_it() {
    let env = Env::new();
    let repo = pushed_feature(&env);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = idle_on(&env, &mut client, on_branch(&repo, "feature-x")).await;
    assert_eq!(client.sessions[&id].phase, PhaseView::Active);
    commit(&client.sessions[&id].worktree, "fix.rs", "fix\n");
    env.gh_lists(&format!("[{}]", pr_json(7, "OPEN", "main", "feature-x")));

    let squash = land(
        &mut client,
        &id,
        Landing::Squash {
            message: "Squash it".into(),
        },
    )
    .await;
    let pushed = land(&mut client, &id, pr_landing()).await;

    let Err(RequestError::Refused { message }) = squash else {
        panic!("expected a refusal: {squash:?}");
    };
    assert!(message.contains("PR #7 is open"), "{message}");
    assert_eq!(
        pushed,
        Ok(Reply::Pushed {
            number: 7,
            committed_changes: false,
        })
    );
    assert!(!env.gh_calls().contains("create"));
    let view = client
        .until(&id, "PR open", |view| view.phase == PhaseView::PrOpen)
        .await;
    assert_eq!(view.flags.pr_number, Some(7));
    let remote = env.path("remote.git");
    assert_eq!(git(&remote, &["show", "feature-x:fix.rs"]), "fix");
}

#[tokio::test]
async fn landing_a_suspended_session_with_an_open_pr_only_pushes_to_it() {
    let env = Env::new();
    let repo = pushed_feature(&env);
    env.gh_reports(&pr_json(7, "OPEN", "main", "feature-x"));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = idle_on(&env, &mut client, on_branch(&repo, "#7")).await;
    commit(&client.sessions[&id].worktree, "fix.rs", "fix\n");
    kill(client.holder_pid(&id).unwrap(), "-KILL");
    client
        .until(&id, "Suspended", |view| view.phase == PhaseView::Suspended)
        .await;

    let squash = land(
        &mut client,
        &id,
        Landing::Squash {
            message: "Squash it".into(),
        },
    )
    .await;
    let pushed = land(&mut client, &id, pr_landing()).await;

    assert!(
        matches!(&squash, Err(RequestError::Refused { message }) if message.contains("PR #7 is open")),
        "{squash:?}"
    );
    assert!(
        matches!(pushed, Ok(Reply::Pushed { number: 7, .. })),
        "{pushed:?}"
    );
    let remote = env.path("remote.git");
    assert_eq!(git(&remote, &["show", "feature-x:fix.rs"]), "fix");
}

#[tokio::test]
async fn following_a_base_that_cannot_be_fetched_leaves_the_base_marked_missing() {
    let env = Env::new();
    let repo = pushed_feature(&env);
    release_only_on_origin(&repo);
    env.gh_reports(&pr_json(7, "OPEN", "release", "feature-x"));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = client.create(on_branch(&repo, "#7")).await;
    client
        .until(&id, "PR open", |view| view.phase == PhaseView::PrOpen)
        .await;
    git(&repo, &["branch", "-D", "release"]);
    client.reconcile().await;
    client
        .until(&id, "Base missing", |view| view.flags.base_missing)
        .await;

    env.gh_reports(&pr_json(7, "OPEN", "ghost", "feature-x"));
    refresh(&mut client, &id).await;

    let view = client
        .until(&id, "following ghost", |view| view.base == "ghost")
        .await;
    assert!(view.flags.base_missing);
    assert!(view.flags.needs_rebase);
}
