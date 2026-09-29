use orch_git::SessionWorktree;

use crate::common::{Fixture, commit, git, name, read, rev};

#[test]
fn a_free_slug_names_the_branch_with_the_prefix() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.repo().unique_name("orch/", "fix-bug"),
        name("fix-bug")
    );
}

#[test]
fn a_taken_branch_gets_a_numeric_suffix() {
    let fixture = Fixture::new();
    git(&fixture.root, &["branch", "orch/fix-bug"]);
    git(&fixture.root, &["branch", "orch/fix-bug-2"]);
    assert_eq!(
        fixture.repo().unique_name("orch/", "fix-bug"),
        name("fix-bug-3")
    );
}

#[test]
fn a_taken_worktree_directory_gets_a_numeric_suffix() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.root.join(".orchestrator/worktrees/fix-bug")).unwrap();
    assert_eq!(
        fixture.repo().unique_name("orch/", "fix-bug"),
        name("fix-bug-2")
    );
}

#[test]
fn worktree_path_is_under_the_orchestrator_directory() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.repo().worktree_path("fix"),
        fixture.root.join(".orchestrator/worktrees/fix")
    );
}

#[test]
fn worktree_is_created_inside_the_repo_on_a_new_branch_from_the_base() {
    let fixture = Fixture::new();
    let base_tip = commit(&fixture.root, "a.txt", "a\n", "a");
    let repo = fixture.repo();

    let worktree = repo.create_worktree(&name("fix-bug"), "main").unwrap();

    assert_eq!(
        worktree,
        SessionWorktree {
            path: fixture.root.join(".orchestrator/worktrees/fix-bug"),
            name: name("fix-bug"),
            base: "main".into(),
        }
    );
    assert_eq!(repo.session_worktree(&name("fix-bug"), "main"), worktree);
    assert_eq!(
        git(&worktree.path, &["symbolic-ref", "--short", "HEAD"]),
        "orch/fix-bug"
    );
    assert_eq!(rev(&worktree.path, "HEAD"), base_tip);
    assert_eq!(read(&worktree.path, "a.txt"), "a\n");
    assert!(repo.worktree_exists(&worktree.path));
}

#[test]
fn worktrees_are_hidden_through_the_local_exclude_file_only() {
    let fixture = Fixture::new();
    fixture.session("one");
    fixture.session("two");

    let exclude = read(&fixture.root, ".git/info/exclude");
    assert_eq!(
        exclude
            .lines()
            .filter(|line| *line == "/.orchestrator/")
            .count(),
        1
    );
    assert_eq!(
        git(
            &fixture.root,
            &["status", "--porcelain", "--untracked-files=all"]
        ),
        ""
    );
    assert!(!fixture.root.join(".gitignore").exists());
}

#[test]
fn exclude_entry_is_appended_after_existing_patterns() {
    let fixture = Fixture::new();
    std::fs::write(fixture.root.join(".git/info/exclude"), "*.log").unwrap();

    fixture.session("one");

    assert_eq!(
        read(&fixture.root, ".git/info/exclude"),
        "*.log\n/.orchestrator/\n"
    );
}

#[test]
fn worktree_from_a_missing_base_fails_without_leaving_anything() {
    let fixture = Fixture::new();
    let repo = fixture.repo();

    assert!(repo.create_worktree(&name("one"), "nope").is_err());

    assert!(!repo.worktree_path("one").exists());
    assert!(!repo.branch_exists("orch/one"));
}

#[test]
fn a_plain_directory_is_not_a_worktree() {
    let fixture = Fixture::new();
    let repo = fixture.repo();
    std::fs::create_dir_all(repo.worktree_path("fake")).unwrap();
    assert!(!repo.worktree_exists(&repo.worktree_path("fake")));
}

#[test]
fn a_worktree_can_be_recreated_for_an_existing_branch() {
    let fixture = Fixture::new();
    let repo = fixture.repo();
    let worktree = fixture.session("work");
    let tip = commit(&worktree.path, "one.txt", "1\n", "mine");
    std::fs::remove_dir_all(&worktree.path).unwrap();

    let recreated = repo.attach_worktree(&name("work")).unwrap();

    assert_eq!(recreated, worktree.path);
    assert_eq!(rev(&recreated, "HEAD"), tip);
    assert_eq!(
        git(&recreated, &["symbolic-ref", "--short", "HEAD"]),
        "orch/work"
    );
}

#[test]
fn the_branch_checked_out_in_a_worktree_is_known_by_its_path() {
    let fixture = Fixture::new();
    let repo = fixture.repo();
    let worktree = repo.create_worktree(&name("fix-bug"), "main").unwrap();

    assert_eq!(
        repo.worktree_branch(&worktree.path).unwrap(),
        Some("orch/fix-bug".to_string())
    );
    assert_eq!(repo.worktree_branch(fixture.outside()).unwrap(), None);
}
