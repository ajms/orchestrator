use std::path::Path;

use orch_git::LandingError;

use crate::common::{Fixture, commit, git, read, rev, write};

fn tree_files(dir: &Path, rev: &str) -> Vec<String> {
    git(dir, &["ls-tree", "-r", "--name-only", rev])
        .lines()
        .map(String::from)
        .collect()
}

fn move_main_checkout_off_base(fixture: &Fixture) {
    git(&fixture.root, &["switch", "-q", "-c", "elsewhere"]);
}

#[test]
fn squash_lands_the_whole_worktree_state_as_one_commit_on_the_base() {
    let fixture = Fixture::new();
    commit(&fixture.root, ".gitignore", "*.log\n", "ignore logs");
    commit(&fixture.root, "gone.txt", "bye\n", "to delete");
    let base_tip = rev(&fixture.root, "main");
    move_main_checkout_off_base(&fixture);
    let worktree = fixture.session("work");
    commit(&worktree.path, "committed.txt", "c\n", "agent commit");
    write(&worktree.path, "README.md", "changed\n");
    write(&worktree.path, "untracked.txt", "u\n");
    write(&worktree.path, "debug.log", "noise\n");
    std::fs::remove_file(worktree.path.join("gone.txt")).unwrap();

    let landed = fixture
        .repo()
        .land_squash(&worktree, "Do the work")
        .unwrap();

    assert_eq!(rev(&fixture.root, "main"), landed.commit);
    assert_eq!(rev(&fixture.root, "main^"), base_tip);
    assert_eq!(
        git(&fixture.root, &["log", "-1", "--format=%B", "main"]),
        "Do the work"
    );
    assert_eq!(
        tree_files(&fixture.root, "main"),
        [".gitignore", "README.md", "committed.txt", "untracked.txt"]
    );
    assert_eq!(git(&fixture.root, &["show", "main:README.md"]), "changed");
}

#[test]
fn squash_leaves_the_worktree_and_its_index_untouched() {
    let fixture = Fixture::new();
    move_main_checkout_off_base(&fixture);
    let worktree = fixture.session("work");
    write(&worktree.path, "staged.txt", "s\n");
    git(&worktree.path, &["add", "staged.txt"]);
    write(&worktree.path, "untracked.txt", "u\n");
    let status_before = git(&worktree.path, &["status", "--porcelain"]);
    let branch_before = rev(&worktree.path, "HEAD");

    fixture.repo().land_squash(&worktree, "msg").unwrap();

    assert_eq!(
        git(&worktree.path, &["status", "--porcelain"]),
        status_before
    );
    assert_eq!(rev(&worktree.path, "HEAD"), branch_before);
}

#[test]
fn tracked_files_matching_ignore_patterns_stay_in_the_squash() {
    let fixture = Fixture::new();
    commit(&fixture.root, ".gitignore", "*.log\n", "ignore logs");
    write(&fixture.root, "keep.log", "kept\n");
    git(&fixture.root, &["add", "-f", "keep.log"]);
    git(&fixture.root, &["commit", "-q", "-m", "tracked log"]);
    move_main_checkout_off_base(&fixture);
    let worktree = fixture.session("work");
    write(&worktree.path, "new.txt", "n\n");

    fixture.repo().land_squash(&worktree, "msg").unwrap();

    assert!(tree_files(&fixture.root, "main").contains(&"keep.log".to_string()));
}

#[test]
fn squash_merges_onto_a_base_that_moved_on() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    let moved_tip = commit(&fixture.root, "base.txt", "b\n", "base moved");
    move_main_checkout_off_base(&fixture);
    write(&worktree.path, "mine.txt", "m\n");

    fixture.repo().land_squash(&worktree, "msg").unwrap();

    assert_eq!(rev(&fixture.root, "main^"), moved_tip);
    assert_eq!(
        tree_files(&fixture.root, "main"),
        ["README.md", "base.txt", "mine.txt"]
    );
}

#[test]
fn a_clean_main_checkout_on_the_base_is_updated_too() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    write(&fixture.root, "scratch.txt", "user's untracked notes\n");
    write(&worktree.path, "README.md", "changed\n");
    write(&worktree.path, "new.txt", "n\n");

    let landed = fixture.repo().land_squash(&worktree, "msg").unwrap();

    assert_eq!(rev(&fixture.root, "HEAD"), landed.commit);
    assert_eq!(read(&fixture.root, "README.md"), "changed\n");
    assert_eq!(read(&fixture.root, "new.txt"), "n\n");
    assert_eq!(
        read(&fixture.root, "scratch.txt"),
        "user's untracked notes\n"
    );
    assert_eq!(
        git(&fixture.root, &["status", "--porcelain"]),
        "?? scratch.txt"
    );
}

#[test]
fn a_dirty_main_checkout_on_the_base_refuses_the_landing() {
    let fixture = Fixture::new();
    let base_tip = rev(&fixture.root, "main");
    let worktree = fixture.session("work");
    write(&fixture.root, "README.md", "user's edit\n");
    write(&worktree.path, "new.txt", "n\n");

    let error = fixture.repo().land_squash(&worktree, "msg").unwrap_err();

    assert_eq!(
        error,
        LandingError::BaseCheckoutDirty {
            worktree: fixture.root.clone()
        }
    );
    assert_eq!(rev(&fixture.root, "main"), base_tip);
    assert_eq!(read(&fixture.root, "README.md"), "user's edit\n");
    assert!(!fixture.root.join("new.txt").exists());
}

#[test]
fn an_untracked_file_in_the_way_in_the_main_checkout_refuses_the_landing() {
    let fixture = Fixture::new();
    let base_tip = rev(&fixture.root, "main");
    let worktree = fixture.session("work");
    write(&fixture.root, "new.txt", "user's own file\n");
    write(&worktree.path, "new.txt", "agent's file\n");

    let error = fixture.repo().land_squash(&worktree, "msg").unwrap_err();

    assert_eq!(
        error,
        LandingError::BaseCheckoutDirty {
            worktree: fixture.root.clone()
        }
    );
    assert_eq!(rev(&fixture.root, "main"), base_tip);
    assert_eq!(read(&fixture.root, "new.txt"), "user's own file\n");
}

#[test]
fn a_base_checked_out_in_another_worktree_refuses_the_landing() {
    let fixture = Fixture::new();
    let lower = fixture.session("lower");
    let upper = fixture.session_on("upper", "orch/lower");
    let lower_tip = rev(&fixture.root, "orch/lower");
    write(&upper.path, "new.txt", "n\n");

    let error = fixture.repo().land_squash(&upper, "msg").unwrap_err();

    assert_eq!(
        error,
        LandingError::BaseCheckedOutElsewhere {
            worktree: lower.path.clone()
        }
    );
    assert_eq!(rev(&fixture.root, "orch/lower"), lower_tip);
    assert!(!lower.path.join("new.txt").exists());
    assert_eq!(git(&lower.path, &["status", "--porcelain"]), "");
}

#[test]
fn a_conflicting_landing_aborts_and_reports_the_conflicting_paths() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    commit(&fixture.root, "README.md", "base version\n", "base edit");
    let base_tip = rev(&fixture.root, "main");
    write(&worktree.path, "README.md", "worktree version\n");
    write(&worktree.path, "fine.txt", "f\n");

    let error = fixture.repo().land_squash(&worktree, "msg").unwrap_err();

    assert_eq!(
        error,
        LandingError::Conflict {
            paths: vec!["README.md".into()]
        }
    );
    assert_eq!(rev(&fixture.root, "main"), base_tip);
    assert_eq!(git(&fixture.root, &["status", "--porcelain"]), "");
    assert_eq!(read(&worktree.path, "README.md"), "worktree version\n");
}

#[test]
fn a_worktree_without_changes_has_nothing_to_land() {
    let fixture = Fixture::new();
    let base_tip = rev(&fixture.root, "main");
    let worktree = fixture.session("work");

    let error = fixture.repo().land_squash(&worktree, "msg").unwrap_err();

    assert_eq!(error, LandingError::NothingToLand);
    assert_eq!(rev(&fixture.root, "main"), base_tip);
}

#[test]
fn landing_onto_a_missing_base_fails() {
    let fixture = Fixture::new();
    let mut worktree = fixture.session("work");
    write(&worktree.path, "new.txt", "n\n");
    worktree.base = "gone".into();

    let error = fixture.repo().land_squash(&worktree, "msg").unwrap_err();

    assert!(matches!(error, LandingError::Failed(_)), "{error:?}");
}

#[test]
fn pushing_a_branch_publishes_it_to_origin_with_upstream() {
    let fixture = Fixture::with_origin("main");
    let worktree = fixture.session("work");
    let tip = commit(&worktree.path, "one.txt", "1\n", "mine");

    fixture.repo().push("orch/work").unwrap();

    assert_eq!(rev(&fixture.origin, "refs/heads/orch/work"), tip);
    assert_eq!(
        git(&fixture.root, &["config", "branch.orch/work.remote"]),
        "origin"
    );
}

#[test]
fn pushing_without_origin_fails() {
    let fixture = Fixture::new();
    git(&fixture.root, &["branch", "orch/work"]);
    assert!(fixture.repo().push("orch/work").is_err());
}
