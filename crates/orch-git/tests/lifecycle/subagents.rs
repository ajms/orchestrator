use orch_git::{Error, InUse, subagent_worktrees_dir};

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
        assert_eq!(
            repo.create_subagent_worktree(&worktree.path, name),
            Err(Error::InvalidSubagentWorktreeName { name: name.into() }),
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

#[test]
fn a_subagent_worktree_named_through_a_symlinked_repo_is_still_removed() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    let repo = fixture.repo();
    let path = repo
        .create_subagent_worktree(&worktree.path, "agent-1")
        .unwrap();
    let link = fixture.outside().join("link");
    std::os::unix::fs::symlink(&fixture.root, &link).unwrap();
    let relative = path.strip_prefix(&fixture.root).unwrap();

    repo.remove_subagent_worktree(&worktree.path, &link.join(relative))
        .unwrap();

    assert!(!path.exists());
}

#[test]
fn a_stray_entry_among_subagent_worktrees_does_not_block_removing_the_session() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    let repo = fixture.repo();
    let subagent = repo
        .create_subagent_worktree(&worktree.path, "agent-1")
        .unwrap();
    write(&subagent_worktrees_dir(&worktree.path), "stray.txt", "x\n");

    repo.remove_session_worktree(&worktree, None).unwrap();

    assert!(!worktree.path.exists());
    assert!(!subagent.exists());
    assert!(!repo.branch_exists("orch/work"));
}

#[test]
fn cleaning_up_after_a_vanished_session_worktree_removes_its_subagent_worktrees() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    let repo = fixture.repo();
    let checkpoint = repo.cleanup_checkpoint(&worktree);
    let subagent = repo
        .create_subagent_worktree(&worktree.path, "agent-1")
        .unwrap();
    std::fs::remove_dir_all(&worktree.path).unwrap();

    repo.finish_session_cleanup(&worktree, &checkpoint, InUse::default(), None)
        .unwrap();

    assert!(!subagent.exists());
    assert!(!git(&fixture.root, &["worktree", "list"]).contains("agent-1"));
}

#[test]
fn repairing_a_moved_sessions_worktree_repairs_its_subagent_worktrees() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    fixture
        .repo()
        .create_subagent_worktree(&worktree.path, "agent-1")
        .unwrap();
    let moved = fixture.outside().join("moved");
    std::fs::rename(&fixture.root, &moved).unwrap();
    let repo = orch_git::Repo::open(&moved).unwrap();
    let session = moved.join(".orchestrator/worktrees/work");
    let subagent = moved.join(".orchestrator/subagents/work/agent-1");

    repo.repair_worktrees(std::slice::from_ref(&session))
        .unwrap();

    assert!(repo.worktree_exists(&subagent));
    assert_eq!(
        git(&subagent, &["branch", "--show-current"]),
        "worktree-agent-1"
    );
}

fn show(fixture: &Fixture, oid: &str, file: &str) -> String {
    git(&fixture.root, &["show", &format!("{oid}:{file}")])
}

fn has_ref(fixture: &Fixture, name: &str) -> bool {
    !git(&fixture.root, &["for-each-ref", name]).is_empty()
}

#[test]
fn diff_refs_cover_a_subagents_committed_work_live_and_after_removal() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    let repo = fixture.repo();
    let base = rev(&worktree.path, "HEAD");
    let path = repo
        .create_subagent_worktree(&worktree.path, "agent-1")
        .unwrap();
    commit(&path, "b.txt", "b\n", "subagent work");

    let live = repo
        .subagent_diff_refs(&worktree.path, "agent-1")
        .unwrap()
        .unwrap();
    assert_eq!(live.base, base);
    assert_eq!(live.branch, "worktree-agent-1");
    assert_eq!(show(&fixture, &live.tip, "b.txt"), "b");

    assert_eq!(
        repo.remove_subagent_worktree(&worktree.path, &path),
        Ok(None)
    );
    let old = repo
        .subagent_diff_refs(&worktree.path, "agent-1")
        .unwrap()
        .unwrap();
    assert_eq!(old.base, base);
    assert_eq!(show(&fixture, &old.tip, "b.txt"), "b");
}

#[test]
fn diff_refs_include_uncommitted_and_untracked_work() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    let repo = fixture.repo();
    let path = repo
        .create_subagent_worktree(&worktree.path, "agent-1")
        .unwrap();
    commit(&path, "tracked.txt", "one\n", "subagent work");
    write(&path, "tracked.txt", "two\n");
    write(&path, "new.txt", "untracked\n");

    repo.remove_subagent_worktree(&worktree.path, &path)
        .unwrap();
    let tip = repo
        .subagent_diff_refs(&worktree.path, "agent-1")
        .unwrap()
        .unwrap()
        .tip;

    assert_eq!(show(&fixture, &tip, "tracked.txt"), "two");
    assert_eq!(show(&fixture, &tip, "new.txt"), "untracked");
}

#[test]
fn a_merged_subagent_branch_is_deleted_but_its_diff_refs_are_kept() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    let repo = fixture.repo();
    let path = repo
        .create_subagent_worktree(&worktree.path, "agent-1")
        .unwrap();
    commit(&path, "b.txt", "b\n", "subagent work");
    git(&worktree.path, &["merge", "--ff-only", "worktree-agent-1"]);

    repo.remove_subagent_worktree(&worktree.path, &path)
        .unwrap();

    assert!(!repo.branch_exists("worktree-agent-1"));
    let tip = repo
        .subagent_diff_refs(&worktree.path, "agent-1")
        .unwrap()
        .unwrap()
        .tip;
    assert_eq!(show(&fixture, &tip, "b.txt"), "b");
}

#[test]
fn removing_a_session_drops_its_subagent_refs() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    let other = fixture.session("work2");
    let repo = fixture.repo();
    repo.create_subagent_worktree(&worktree.path, "agent-1")
        .unwrap();
    repo.create_subagent_worktree(&other.path, "agent-2")
        .unwrap();

    repo.remove_session_worktree(&worktree, None).unwrap();

    assert!(!has_ref(&fixture, "refs/orch/subagents/work/"));
    assert!(has_ref(&fixture, "refs/orch/subagents/work2/"));
}

#[test]
fn forged_or_garbage_subagent_refs_are_ignored() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    let repo = fixture.repo();
    let path = repo
        .create_subagent_worktree(&worktree.path, "agent-1")
        .unwrap();
    repo.remove_subagent_worktree(&worktree.path, &path)
        .unwrap();
    let tree = git(&fixture.root, &["rev-parse", "HEAD^{tree}"]);
    let orphan = git(&fixture.root, &["commit-tree", &tree, "-m", "forged"]);

    git(
        &fixture.root,
        &[
            "update-ref",
            "refs/orch/subagents/work/agent-1/base",
            &orphan,
        ],
    );
    assert_eq!(repo.subagent_diff_refs(&worktree.path, "agent-1"), Ok(None));

    git(
        &fixture.root,
        &["update-ref", "refs/orch/subagents/work/agent-1/base", &tree],
    );
    assert_eq!(repo.subagent_diff_refs(&worktree.path, "agent-1"), Ok(None));

    assert_eq!(repo.subagent_diff_refs(&worktree.path, "missing"), Ok(None));
    assert!(repo.subagent_diff_refs(&worktree.path, "../x").is_err());
}
