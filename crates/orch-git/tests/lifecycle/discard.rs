use orch_git::Commit;

use crate::common::{Fixture, commit, git, write};

fn subjects(commits: &[Commit]) -> Vec<&str> {
    commits
        .iter()
        .map(|commit| commit.subject.as_str())
        .collect()
}

#[test]
fn discard_preview_lists_uncommitted_files_and_unlanded_commits() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    let first = commit(&worktree.path, "one.txt", "1\n", "first change");
    commit(&worktree.path, "two.txt", "2\n", "second change");
    write(&worktree.path, "README.md", "modified\n");
    write(&worktree.path, "staged.txt", "s\n");
    git(&worktree.path, &["add", "staged.txt"]);
    write(&worktree.path, "dir/untracked.txt", "u\n");
    std::fs::remove_file(worktree.path.join("one.txt")).unwrap();

    let preview = fixture.repo().discard_preview(&worktree).unwrap();

    assert_eq!(
        preview.uncommitted,
        ["README.md", "dir/untracked.txt", "one.txt", "staged.txt"]
    );
    assert_eq!(
        subjects(&preview.unlanded),
        ["second change", "first change"]
    );
    assert_eq!(preview.unlanded[1].id, first);
}

#[test]
fn commits_already_on_the_base_are_not_unlanded() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    commit(&worktree.path, "one.txt", "1\n", "shared");
    git(&fixture.root, &["merge", "-q", "--ff-only", "orch/work"]);
    commit(&worktree.path, "two.txt", "2\n", "only here");

    let preview = fixture.repo().discard_preview(&worktree).unwrap();

    assert!(preview.uncommitted.is_empty());
    assert_eq!(subjects(&preview.unlanded), ["only here"]);
}

#[test]
fn with_the_base_gone_unlanded_commits_are_those_on_no_other_branch() {
    let fixture = Fixture::new();
    git(&fixture.root, &["branch", "feature"]);
    let worktree = fixture.session_on("work", "feature");
    commit(&worktree.path, "one.txt", "1\n", "mine");
    git(&fixture.root, &["branch", "-D", "feature"]);

    let preview = fixture.repo().discard_preview(&worktree).unwrap();

    assert_eq!(subjects(&preview.unlanded), ["mine"]);
}

#[test]
fn discard_preview_of_a_vanished_worktree_still_lists_commits() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    commit(&worktree.path, "one.txt", "1\n", "mine");
    std::fs::remove_dir_all(&worktree.path).unwrap();

    let preview = fixture.repo().discard_preview(&worktree).unwrap();

    assert!(preview.uncommitted.is_empty());
    assert_eq!(subjects(&preview.unlanded), ["mine"]);
}
