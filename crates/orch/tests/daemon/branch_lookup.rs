use std::path::{Path, PathBuf};

use orch_protocol::{BranchFound, BranchLookup, LookupBranch, LookupDepth, OpenPr, Reply, Request};

use crate::common::*;

const OPEN_PR: &str = r#"{"number":7,"title":"Add the feature","state":"OPEN","baseRefName":"release","headRefName":"feature-x","isCrossRepository":false}"#;

struct Fixture {
    env: Env,
    repo: PathBuf,
    remote: PathBuf,
}

fn fixture() -> Fixture {
    let env = Env::new();
    let repo = env.repo("app");
    let remote = env.github_remote(&repo);
    env.fake_gh();
    env.gh_reports(OPEN_PR);
    feature_branch(&repo);
    git(&repo, &["push", "-q", "origin", "feature-x"]);
    git(&repo, &["branch", "remote-y"]);
    git(&repo, &["push", "-q", "origin", "remote-y"]);
    git(&repo, &["branch", "-D", "remote-y"]);
    git(&repo, &["update-ref", "-d", "refs/remotes/origin/remote-y"]);
    git(&repo, &["remote", "set-head", "origin", "main"]);
    Fixture { env, repo, remote }
}

async fn look_up(
    client: &mut TestClient,
    repo: &Path,
    branch: &str,
    depth: LookupDepth,
) -> BranchLookup {
    let request = Request::LookupBranch(LookupBranch {
        repo: repo.to_path_buf(),
        branch: branch.into(),
        base: None,
        depth,
    });
    match client.request(request).await {
        Ok(Reply::BranchLookup(lookup)) => lookup,
        other => panic!("expected a lookup, got {other:?}"),
    }
}

fn main_refused() -> BranchLookup {
    BranchLookup::Refused {
        reason: "main is the Session's Base branch".into(),
    }
}

#[tokio::test]
async fn an_origin_lookup_reports_a_new_branch_an_existing_branch_and_a_pr() {
    let Fixture { env, repo, .. } = fixture();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    let fresh = look_up(&mut client, &repo, "fresh", LookupDepth::Origin).await;
    let local = look_up(&mut client, &repo, "feature-x", LookupDepth::Origin).await;
    let on_origin = look_up(&mut client, &repo, "remote-y", LookupDepth::Origin).await;
    let pr = look_up(&mut client, &repo, "#7", LookupDepth::Origin).await;
    let refused = look_up(&mut client, &repo, "main", LookupDepth::Origin).await;

    assert_eq!(
        fresh,
        BranchLookup::New {
            base: "main".into()
        }
    );
    assert_eq!(
        local,
        BranchLookup::Existing {
            branch: "feature-x".into(),
            found: BranchFound::Local,
            pr: None,
        }
    );
    assert_eq!(
        on_origin,
        BranchLookup::Existing {
            branch: "remote-y".into(),
            found: BranchFound::Origin,
            pr: None,
        }
    );
    assert_eq!(
        pr,
        BranchLookup::Existing {
            branch: "feature-x".into(),
            found: BranchFound::Local,
            pr: Some(OpenPr {
                number: 7,
                title: "Add the feature".into(),
                base: "release".into(),
            }),
        }
    );
    assert_eq!(refused, main_refused());
}

#[tokio::test]
async fn a_local_lookup_refuses_without_asking_origin_or_github() {
    let Fixture { env, repo, remote } = fixture();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    git(&remote, &["symbolic-ref", "HEAD", "refs/heads/feature-x"]);

    let only_on_origin = look_up(&mut client, &repo, "remote-y", LookupDepth::Local).await;
    let pr = look_up(&mut client, &repo, "#7", LookupDepth::Local).await;
    let refused = look_up(&mut client, &repo, "main", LookupDepth::Local).await;

    assert_eq!(only_on_origin, BranchLookup::NotRefusedLocally);
    assert_eq!(pr, BranchLookup::NotRefusedLocally);
    assert_eq!(refused, main_refused());
    assert_eq!(
        git(&repo, &["for-each-ref", "refs/remotes/origin/remote-y"]),
        ""
    );
    assert_eq!(
        git(&repo, &["symbolic-ref", "refs/remotes/origin/HEAD"]),
        "refs/remotes/origin/main"
    );
    assert_eq!(env.gh_calls(), "");
}

#[tokio::test]
async fn a_lookup_creates_no_local_branch_and_no_worktree() {
    let Fixture { env, repo, remote } = fixture();
    git(&repo, &["push", "-q", "origin", "main:trunk"]);
    git(&remote, &["symbolic-ref", "HEAD", "refs/heads/trunk"]);
    git(&repo, &["remote", "set-head", "origin", "trunk"]);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    let fresh = look_up(&mut client, &repo, "fresh", LookupDepth::Origin).await;
    look_up(&mut client, &repo, "remote-y", LookupDepth::Origin).await;
    look_up(&mut client, &repo, "fresh", LookupDepth::Local).await;

    assert_eq!(
        fresh,
        BranchLookup::New {
            base: "trunk".into()
        }
    );
    assert_eq!(
        git(&repo, &["branch", "--list", "remote-y", "fresh", "trunk"]),
        ""
    );
    assert!(!repo.join(".orchestrator/worktrees").exists());
    assert!(client.session_list().await.is_empty());
}
