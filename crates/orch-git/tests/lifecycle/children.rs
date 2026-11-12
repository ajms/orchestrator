use orch_git::LandingError;

use crate::common::{Fixture, commit, git, name, read, rev, write};

fn parent_and_child(fixture: &Fixture) -> (orch_git::SessionWorktree, orch_git::SessionWorktree) {
    let parent = fixture.session("parent");
    commit(&parent.path, "shared.txt", "one\n", "parent work");
    let child = fixture
        .repo()
        .create_child_worktree(&name("child"), &parent.name.branch)
        .unwrap();
    (parent, child)
}

#[test]
fn a_child_branches_from_its_parents_branch() {
    let fixture = Fixture::new();
    let (parent, child) = parent_and_child(&fixture);
    assert_eq!(child.base, parent.name.branch);
    assert_eq!(rev(&child.path, "HEAD"), rev(&parent.path, "HEAD"));
    assert_eq!(read(&child.path, "shared.txt"), "one\n");
}

#[test]
fn landing_a_child_squashes_onto_the_parent_and_keeps_its_uncommitted_work() {
    let fixture = Fixture::new();
    let (parent, child) = parent_and_child(&fixture);
    let parent_tip = rev(&parent.path, "HEAD");
    commit(&child.path, "child.txt", "c\n", "child commit");
    write(&child.path, "loose.txt", "l\n");
    write(&parent.path, "shared.txt", "edited by the parent\n");
    write(&parent.path, "notes.txt", "untracked\n");

    let landed = fixture
        .repo()
        .land_child(&child, &parent.path, "Child work")
        .unwrap();

    assert_eq!(rev(&parent.path, "HEAD"), landed.commit);
    assert_eq!(rev(&parent.path, "HEAD^"), parent_tip);
    assert_eq!(read(&parent.path, "child.txt"), "c\n");
    assert_eq!(read(&parent.path, "loose.txt"), "l\n");
    assert_eq!(read(&parent.path, "shared.txt"), "edited by the parent\n");
    assert_eq!(read(&parent.path, "notes.txt"), "untracked\n");
    let status = git(&parent.path, &["status", "--porcelain"]);
    assert!(status.contains(" M shared.txt"), "{status}");
    assert!(status.contains("?? notes.txt"), "{status}");
    assert!(!status.contains("child.txt"), "{status}");
}

#[test]
fn landing_a_child_refuses_when_the_parent_has_uncommitted_changes_in_the_same_files() {
    let fixture = Fixture::new();
    let (parent, child) = parent_and_child(&fixture);
    let parent_tip = rev(&parent.path, "HEAD");
    write(&child.path, "shared.txt", "child edit\n");
    write(&child.path, "new.txt", "n\n");
    write(&parent.path, "shared.txt", "parent edit\n");
    write(&parent.path, "new.txt", "parent's own\n");

    let err = fixture
        .repo()
        .land_child(&child, &parent.path, "Child work")
        .unwrap_err();

    assert_eq!(
        err,
        LandingError::ParentDirty {
            paths: vec!["new.txt".into(), "shared.txt".into()]
        }
    );
    assert_eq!(rev(&parent.path, "HEAD"), parent_tip);
    assert_eq!(read(&parent.path, "shared.txt"), "parent edit\n");
    assert_eq!(read(&parent.path, "new.txt"), "parent's own\n");
}

#[test]
fn landing_a_child_reports_conflicts_with_the_parents_branch() {
    let fixture = Fixture::new();
    let (parent, child) = parent_and_child(&fixture);
    commit(&child.path, "shared.txt", "child\n", "child");
    commit(&parent.path, "shared.txt", "parent\n", "parent");
    let parent_tip = rev(&parent.path, "HEAD");

    let err = fixture
        .repo()
        .land_child(&child, &parent.path, "Child work")
        .unwrap_err();

    assert_eq!(
        err,
        LandingError::Conflict {
            paths: vec!["shared.txt".into()]
        }
    );
    assert_eq!(rev(&parent.path, "HEAD"), parent_tip);
}
