use orch_git::{Error, Leftover};

use crate::common::{Fixture, git, name};

#[test]
fn leftovers_are_orchestrator_worktrees_and_branches_without_a_session() {
    let fixture = Fixture::new();
    let repo = fixture.repo();
    fixture.session("known");
    fixture.session("forgotten");
    git(&fixture.root, &["branch", "orch/branch-only"]);
    git(&fixture.root, &["branch", "feature/not-ours"]);
    std::fs::create_dir_all(repo.worktree_path("stray-dir")).unwrap();

    let leftovers = repo.leftovers("orch/", &[name("known")]).unwrap();

    assert_eq!(
        leftovers,
        [
            Leftover::Worktree(repo.worktree_path("forgotten")),
            Leftover::Worktree(repo.worktree_path("stray-dir")),
            Leftover::Branch("orch/branch-only".into()),
            Leftover::Branch("orch/forgotten".into()),
        ]
    );
}

#[test]
fn a_repo_without_orchestrator_state_has_no_leftovers() {
    let fixture = Fixture::new();
    assert!(fixture.repo().leftovers("orch/", &[]).unwrap().is_empty());
}

#[test]
fn removing_leftovers_deletes_registered_and_stray_worktrees_and_branches() {
    let fixture = Fixture::new();
    let repo = fixture.repo();
    fixture.session("forgotten");
    std::fs::create_dir_all(repo.worktree_path("stray-dir")).unwrap();

    for leftover in repo.leftovers("orch/", &[]).unwrap() {
        repo.remove_leftover(&leftover, None).unwrap();
    }

    assert!(repo.leftovers("orch/", &[]).unwrap().is_empty());
    assert!(!git(&fixture.root, &["worktree", "list"]).contains("forgotten"));
}

#[test]
fn a_directory_outside_the_worktrees_directory_is_never_removed_as_a_leftover() {
    let fixture = Fixture::new();
    let outside = fixture.outside().join("precious");
    std::fs::create_dir(&outside).unwrap();

    let error = fixture
        .repo()
        .remove_leftover(&Leftover::Worktree(outside.clone()), None)
        .unwrap_err();

    assert!(
        matches!(error, Error::NotAnOrchestratorWorktree { .. }),
        "{error:?}"
    );
    assert!(outside.exists());
}
