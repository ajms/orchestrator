use orch_git::{Error, subagent_worktrees_dir};

use crate::common::{Fixture, commit, git, rev, write};

#[test]
fn a_sessions_subagent_worktrees_live_beside_the_orchestrator_worktrees() {
    let fixture = Fixture::new();
    let repo = fixture.repo();
    let worktree = fixture.session("work");

    assert_eq!(
        subagent_worktrees_dir(&worktree.path),
        repo.root().join(".orchestrator/subagents/work")
    );
}

#[test]
fn a_subagent_worktree_starts_from_the_sessions_head_on_its_own_branch() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    let head = commit(&worktree.path, "a.txt", "a\n", "session work");

    let path = fixture
        .repo()
        .create_subagent_worktree(&worktree.path, "agent-1")
        .unwrap();

    assert_eq!(path, subagent_worktrees_dir(&worktree.path).join("agent-1"));
    assert_eq!(rev(&path, "HEAD"), head);
    assert_eq!(
        git(&path, &["branch", "--show-current"]),
        "worktree-agent-1"
    );
}

#[test]
fn a_subagent_worktree_name_cannot_escape_the_sessions_subagent_dir() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    let repo = fixture.repo();

    for name in ["../escape", "a/b", "", "."] {
        assert!(
            repo.create_subagent_worktree(&worktree.path, name).is_err(),
            "{name:?}"
        );
    }
}

#[test]
fn removing_a_subagent_worktree_without_new_commits_drops_its_branch() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    let repo = fixture.repo();
    let path = repo
        .create_subagent_worktree(&worktree.path, "agent-1")
        .unwrap();
    write(&path, "scratch.txt", "uncommitted\n");

    repo.remove_subagent_worktree(&worktree.path, &path)
        .unwrap();

    assert!(!path.exists());
    assert!(!repo.branch_exists("worktree-agent-1"));
}

#[test]
fn removing_a_subagent_worktree_keeps_a_branch_with_unmerged_commits() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    let repo = fixture.repo();
    let path = repo
        .create_subagent_worktree(&worktree.path, "agent-1")
        .unwrap();
    let tip = commit(&path, "b.txt", "b\n", "subagent work");

    repo.remove_subagent_worktree(&worktree.path, &path)
        .unwrap();

    assert!(!path.exists());
    assert_eq!(rev(&fixture.root, "worktree-agent-1"), tip);
}

#[test]
fn removing_a_path_outside_the_sessions_subagent_dir_is_refused() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    let other = fixture.session("other");
    let repo = fixture.repo();
    let theirs = repo
        .create_subagent_worktree(&other.path, "agent-1")
        .unwrap();

    for path in [&theirs, &other.path, &worktree.path] {
        assert!(matches!(
            repo.remove_subagent_worktree(&worktree.path, path),
            Err(Error::NotAnOrchestratorWorktree { .. })
        ));
        assert!(path.exists());
    }
}

#[test]
fn removing_a_session_removes_its_subagent_worktrees() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    let repo = fixture.repo();
    let merged = repo
        .create_subagent_worktree(&worktree.path, "agent-1")
        .unwrap();
    let unmerged = repo
        .create_subagent_worktree(&worktree.path, "agent-2")
        .unwrap();
    let tip = commit(&unmerged, "b.txt", "b\n", "subagent work");

    repo.remove_session_worktree(&worktree, None).unwrap();

    assert!(!merged.exists());
    assert!(!unmerged.exists());
    assert!(!subagent_worktrees_dir(&worktree.path).exists());
    assert!(!repo.branch_exists("worktree-agent-1"));
    assert_eq!(rev(&fixture.root, "worktree-agent-2"), tip);
    assert!(!git(&fixture.root, &["worktree", "list"]).contains("agent-"));
}
