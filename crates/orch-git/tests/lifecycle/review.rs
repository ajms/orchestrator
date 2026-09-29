use crate::common::{Fixture, commit, git, rev, write};

#[test]
fn the_review_snapshot_is_exactly_what_a_squash_lands() {
    let fixture = Fixture::new();
    commit(&fixture.root, ".gitignore", "*.log\n", "ignore logs");
    git(&fixture.root, &["switch", "-q", "-c", "elsewhere"]);
    let worktree = fixture.session("work");
    commit(&worktree.path, "committed.txt", "c\n", "agent commit");
    write(&worktree.path, "README.md", "changed\n");
    write(&worktree.path, "untracked.txt", "u\n");
    write(&worktree.path, "debug.log", "noise\n");
    let status_before = git(&worktree.path, &["status", "--porcelain"]);

    let snapshot = fixture.repo().review_snapshot(&worktree).unwrap();

    assert_eq!(
        git(&worktree.path, &["status", "--porcelain"]),
        status_before
    );
    assert_eq!(snapshot.merge_base, rev(&fixture.root, "main"));
    let landed = fixture.repo().land_squash(&worktree, "Land").unwrap();
    assert_eq!(
        snapshot.tree,
        rev(&fixture.root, &format!("{}^{{tree}}", landed.commit))
    );
}

#[test]
fn the_review_diff_runs_from_the_merge_base_and_ignores_base_progress() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    write(&worktree.path, "untracked.txt", "u\n");
    commit(&fixture.root, "later.txt", "main moved\n", "main moves on");

    let repo = fixture.repo();
    let snapshot = repo.review_snapshot(&worktree).unwrap();
    let diff = repo.diff(&snapshot.merge_base, &snapshot.tree).unwrap();

    assert!(
        diff.contains("diff --git a/untracked.txt b/untracked.txt"),
        "{diff}"
    );
    assert!(!diff.contains("later.txt"), "{diff}");
}
