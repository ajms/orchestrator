use crate::common::{Fixture, commit, git};

#[test]
fn branch_contains_base_tip_until_the_base_moves_and_again_after_rebase() {
    let fixture = Fixture::new();
    let worktree = fixture.session("work");
    let repo = fixture.repo();
    commit(&worktree.path, "one.txt", "1\n", "mine");
    assert!(repo.contains_base_tip(&worktree).unwrap());

    commit(&fixture.root, "base.txt", "b\n", "base moved");
    assert!(!repo.contains_base_tip(&worktree).unwrap());

    git(&worktree.path, &["rebase", "-q", "main"]);
    assert!(repo.contains_base_tip(&worktree).unwrap());
}

#[test]
fn contains_base_tip_fails_for_a_missing_base() {
    let fixture = Fixture::new();
    let mut worktree = fixture.session("work");
    worktree.base = "gone".into();
    assert!(fixture.repo().contains_base_tip(&worktree).is_err());
}
