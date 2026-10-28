use orch_git::{BranchSource, Error, InUse, SessionName, SessionWorktree};

use crate::common::{Fixture, commit, git, rev, write};

fn existing(slug: &str, branch: &str) -> SessionName {
    SessionName {
        slug: slug.into(),
        branch: branch.into(),
    }
}

fn source(fixture: &Fixture, branch: &str) -> BranchSource {
    fixture.repo().resolve_branch(branch).unwrap().source
}

fn start(fixture: &Fixture, branch: &str) -> SessionWorktree {
    let repo = fixture.repo();
    let resolved = repo.resolve_branch(branch).unwrap();
    repo.attach_existing_branch(&existing("feature-x", branch), "main", &resolved)
        .unwrap()
}

#[test]
fn a_name_that_exists_nowhere_is_a_new_branch() {
    let fixture = Fixture::with_origin("main");
    assert_eq!(source(&fixture, "feature-x"), BranchSource::New);
}

#[test]
fn a_session_on_a_local_branch_starts_at_its_tip_and_keeps_it_when_removed() {
    let fixture = Fixture::new();
    git(&fixture.root, &["checkout", "-q", "-b", "feature-x"]);
    let tip = commit(&fixture.root, "x.txt", "x\n", "feature work");
    git(&fixture.root, &["checkout", "-q", "main"]);
    let repo = fixture.repo();
    assert_eq!(source(&fixture, "feature-x"), BranchSource::Local);

    let worktree = start(&fixture, "feature-x");

    assert!(worktree.existing_branch);
    assert_eq!(rev(&worktree.path, "HEAD"), tip);
    assert_eq!(
        git(&worktree.path, &["symbolic-ref", "--short", "HEAD"]),
        "feature-x"
    );
    repo.remove_session_worktree(&worktree, None).unwrap();
    assert!(!worktree.path.exists());
    assert_eq!(rev(&fixture.root, "refs/heads/feature-x"), tip);
}

#[test]
fn finishing_an_interrupted_cleanup_keeps_an_existing_branch() {
    let fixture = Fixture::new();
    git(&fixture.root, &["branch", "feature-x"]);
    let worktree = start(&fixture, "feature-x");
    let repo = fixture.repo();
    let checkpoint = repo.cleanup_checkpoint(&worktree);

    let cleaned = repo
        .finish_session_cleanup(&worktree, &checkpoint, InUse::default(), None)
        .unwrap();

    assert!(!worktree.path.exists());
    assert!(repo.branch_exists("feature-x"));
    assert!(!cleaned.kept_worktree && !cleaned.kept_branch);
}

#[test]
fn the_discard_preview_of_an_existing_branch_keeps_it_and_lists_only_uncommitted_files() {
    let fixture = Fixture::new();
    git(&fixture.root, &["branch", "feature-x"]);
    let worktree = start(&fixture, "feature-x");
    commit(&worktree.path, "done.txt", "d\n", "already on the PR");
    write(&worktree.path, "scratch.txt", "s\n");

    let preview = fixture.repo().discard_preview(&worktree).unwrap();

    assert!(preview.branch_kept);
    assert_eq!(preview.uncommitted, ["scratch.txt"]);
    assert!(preview.unlanded.is_empty());
}

#[test]
fn a_branch_only_on_origin_is_fetched_into_a_local_branch_tracking_it() {
    let fixture = Fixture::with_origin("main");
    git(&fixture.root, &["branch", "feature-x"]);
    git(&fixture.root, &["push", "-q", "origin", "feature-x"]);
    git(&fixture.root, &["branch", "-D", "feature-x"]);
    let upstream_tip = fixture.push_upstream("feature-x", "up.txt", "up\n");
    assert_eq!(source(&fixture, "feature-x"), BranchSource::Origin);

    let worktree = start(&fixture, "feature-x");

    assert_eq!(rev(&worktree.path, "HEAD"), upstream_tip);
    assert_eq!(
        git(&fixture.root, &["config", "branch.feature-x.remote"]),
        "origin"
    );
}

#[test]
fn a_local_branch_behind_origin_is_fast_forwarded() {
    let fixture = Fixture::with_origin("main");
    git(&fixture.root, &["branch", "feature-x"]);
    git(&fixture.root, &["push", "-q", "origin", "feature-x"]);
    let upstream_tip = fixture.push_upstream("feature-x", "up.txt", "up\n");
    assert_eq!(source(&fixture, "feature-x"), BranchSource::Local);

    let worktree = start(&fixture, "feature-x");

    assert_eq!(rev(&worktree.path, "HEAD"), upstream_tip);
    assert_eq!(rev(&fixture.root, "refs/heads/feature-x"), upstream_tip);
}

#[test]
fn a_local_branch_ahead_of_origin_starts_as_it_is() {
    let fixture = Fixture::with_origin("main");
    git(&fixture.root, &["checkout", "-q", "-b", "feature-x"]);
    git(&fixture.root, &["push", "-q", "origin", "feature-x"]);
    let local_tip = commit(&fixture.root, "mine.txt", "mine\n", "not pushed");
    git(&fixture.root, &["checkout", "-q", "main"]);

    let worktree = start(&fixture, "feature-x");

    assert_eq!(rev(&worktree.path, "HEAD"), local_tip);
}

#[test]
fn a_local_branch_diverged_from_origin_is_refused() {
    let fixture = Fixture::with_origin("main");
    git(&fixture.root, &["checkout", "-q", "-b", "feature-x"]);
    git(&fixture.root, &["push", "-q", "origin", "feature-x"]);
    let local_tip = commit(&fixture.root, "mine.txt", "mine\n", "not pushed");
    git(&fixture.root, &["checkout", "-q", "main"]);
    fixture.push_upstream("feature-x", "up.txt", "up\n");
    let repo = fixture.repo();
    let diverged = Error::BranchDivergedFromOrigin {
        branch: "feature-x".into(),
    };

    assert_eq!(repo.resolve_branch("feature-x").unwrap_err(), diverged);
    assert_eq!(rev(&fixture.root, "refs/heads/feature-x"), local_tip);
    assert!(!repo.worktree_path("feature-x").exists());
}

#[test]
fn an_unreachable_origin_refuses_the_branch() {
    let fixture = Fixture::with_origin("main");
    git(&fixture.root, &["branch", "feature-x"]);
    std::fs::remove_dir_all(&fixture.origin).unwrap();

    let error = fixture.repo().resolve_branch("feature-x").unwrap_err();

    assert_eq!(error, Error::OriginUnreachable);
    assert_eq!(error.to_string(), "origin can't be reached");
}

#[test]
fn without_an_origin_the_current_branch_is_reserved() {
    let fixture = Fixture::new();
    git(&fixture.root, &["checkout", "-q", "-b", "trunk"]);

    assert_eq!(fixture.repo().reserved_branches(None).unwrap(), ["trunk"]);
}

#[test]
fn with_an_origin_whose_default_is_unknown_the_current_branch_is_not_reserved() {
    let fixture = Fixture::with_origin("main");
    git(
        &fixture.origin,
        &["symbolic-ref", "HEAD", "refs/heads/gone"],
    );
    git(&fixture.root, &["remote", "set-head", "origin", "--delete"]);
    git(&fixture.root, &["checkout", "-q", "-b", "work"]);

    assert!(fixture.repo().reserved_branches(None).unwrap().is_empty());
}

#[test]
fn a_branch_checked_out_in_the_main_checkout_is_refused_naming_it() {
    let fixture = Fixture::new();
    git(&fixture.root, &["checkout", "-q", "-b", "feature-x"]);

    let error = fixture.repo().resolve_branch("feature-x").unwrap_err();

    assert_eq!(
        error,
        Error::BranchInMainCheckout {
            branch: "feature-x".into(),
        }
    );
}

#[test]
fn a_branch_checked_out_in_a_subagent_worktree_is_refused_naming_it() {
    let fixture = Fixture::new();
    let session = fixture.session("work");
    let repo = fixture.repo();
    let subagent = repo
        .create_subagent_worktree(&session.path, "agent-1")
        .unwrap();

    let error = repo.resolve_branch("worktree-agent-1").unwrap_err();

    assert_eq!(
        error,
        Error::BranchCheckedOut {
            branch: "worktree-agent-1".into(),
            worktree: subagent.clone(),
        }
    );
    assert!(
        error.to_string().contains(&subagent.display().to_string()),
        "{error}"
    );
}

#[test]
fn the_slug_of_a_branch_strips_the_prefix_and_keeps_the_name() {
    let fixture = Fixture::new();
    git(&fixture.root, &["branch", "orch/leftover"]);
    std::fs::create_dir_all(fixture.root.join(".orchestrator/worktrees/feature-x")).unwrap();
    let repo = fixture.repo();

    assert_eq!(
        repo.name_for_branch("orch/", "orch/leftover"),
        existing("leftover", "orch/leftover")
    );
    assert_eq!(
        repo.name_for_branch("orch/", "feature-x"),
        existing("feature-x-2", "feature-x")
    );
}
