mod common;

use common::{Fixture, git};
use orch_core::Phase;
use orch_store::{RepoRoot, Store, StoreError};

#[test]
fn repo_identity_is_the_main_checkout_from_anywhere_inside_it() {
    let fx = Fixture::new();
    let main = fx.git_repo("proj");
    std::fs::create_dir_all(main.join("src/deep")).unwrap();
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "side",
            ".orchestrator/worktrees/side",
        ],
    );

    for inside in [
        main.clone(),
        main.join("src/deep"),
        main.join(".orchestrator/worktrees/side"),
    ] {
        assert_eq!(
            RepoRoot::resolve(&inside).unwrap().path(),
            main,
            "{inside:?}"
        );
    }
}

#[test]
fn repo_identity_resolves_symlinks_to_the_real_path() {
    let fx = Fixture::new();
    let main = fx.git_repo("proj");
    let link = fx.dir.path().join("link");
    std::os::unix::fs::symlink(&main, &link).unwrap();
    assert_eq!(RepoRoot::resolve(&link).unwrap().path(), main);
}

#[test]
fn a_path_outside_any_git_repository_is_not_a_repo() {
    let fx = Fixture::new();
    let plain = fx.dir.path().join("plain");
    std::fs::create_dir(&plain).unwrap();
    assert!(RepoRoot::resolve(&plain).is_err());
    assert!(RepoRoot::resolve(&fx.dir.path().join("nowhere")).is_err());
}

#[test]
fn registering_twice_keeps_one_repo_and_lists_most_recently_used_first() {
    let mut fx = Fixture::new();
    let a = fx.register("a");
    let b = fx.register("b");
    assert_eq!(paths(&fx.store), vec![b.path.clone(), a.path.clone()]);

    let again = fx
        .store
        .register_repo(&RepoRoot::resolve(&a.path).unwrap())
        .unwrap();
    assert_eq!(again.id, a.id);
    assert_eq!(paths(&fx.store), vec![a.path, b.path]);
}

#[test]
fn registration_records_the_main_checkouts_head_branch_once() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    assert_eq!(repo.head_branch_at_registration.as_deref(), Some("main"));

    git(&repo.path, &["switch", "-q", "-c", "elsewhere"]);
    let again = fx
        .store
        .register_repo(&RepoRoot::resolve(&repo.path).unwrap())
        .unwrap();
    assert_eq!(again.head_branch_at_registration.as_deref(), Some("main"));
}

#[test]
fn a_detached_main_checkout_records_no_head_branch() {
    let mut fx = Fixture::new();
    let path = fx.git_repo("detached");
    git(&path, &["switch", "-q", "--detach"]);
    let repo = fx
        .store
        .register_repo(&RepoRoot::resolve(&path).unwrap())
        .unwrap();
    assert_eq!(repo.head_branch_at_registration, None);
}

#[test]
fn a_repo_whose_path_is_gone_is_flagged_missing() {
    let mut fx = Fixture::new();
    let repo = fx.register("gone");
    assert!(!fx.store.repo(repo.id).unwrap().unwrap().missing);
    std::fs::remove_dir_all(&repo.path).unwrap();
    assert!(fx.store.repo(repo.id).unwrap().unwrap().missing);
    assert!(fx.store.repos().unwrap()[0].missing);
}

#[test]
fn registry_survives_reopening_the_store() {
    let mut fx = Fixture::new();
    let repo = fx.register("kept");
    let db = fx.db_path();
    drop(fx.store);
    let store = Store::open(&db).unwrap();
    assert_eq!(
        store.repo_by_path(&repo.path).unwrap().map(|r| r.id),
        Some(repo.id)
    );
}

#[test]
fn moving_a_repo_keeps_its_identity_and_relocates_its_worktrees() {
    let mut fx = Fixture::new();
    let repo = fx.register("old");
    let session = fx.session(&repo, "s");
    let moved_to = fx.dir.path().join("new");
    std::fs::rename(&repo.path, &moved_to).unwrap();
    assert!(fx.store.repo(repo.id).unwrap().unwrap().missing);

    let moved = fx
        .store
        .move_repo(repo.id, &RepoRoot::resolve(&moved_to).unwrap())
        .unwrap();
    let moved_to = moved_to.canonicalize().unwrap();
    assert_eq!(moved.id, repo.id);
    assert_eq!(moved.path, moved_to);
    assert!(!moved.missing);
    assert_eq!(fx.store.repo_by_path(&repo.path).unwrap(), None);
    assert_eq!(
        fx.store.session(&session.id).unwrap().unwrap().worktree,
        moved_to.join(".orchestrator/worktrees/s")
    );
}

#[test]
fn moving_onto_another_registered_repo_is_refused() {
    let mut fx = Fixture::new();
    let a = fx.register("a");
    let b = fx.register("b");
    let err = fx
        .store
        .move_repo(a.id, &RepoRoot::resolve(&b.path).unwrap());
    assert!(matches!(err, Err(StoreError::RepoPathTaken)));
}

#[test]
fn forgetting_a_repo_is_refused_while_it_has_unfinished_sessions() {
    let mut fx = Fixture::new();
    let repo = fx.register("busy");
    let mut active = fx.session(&repo, "active");
    let mut suspended = fx.session(&repo, "suspended");
    suspended.phase = Phase::Suspended;
    fx.store.save_session(&suspended).unwrap();

    assert!(matches!(
        fx.store.forget_repo(repo.id),
        Err(StoreError::LiveSessions { count: 2 })
    ));

    active.phase = Phase::Landed;
    suspended.phase = Phase::Discarded;
    fx.store.save_session(&active).unwrap();
    fx.store.save_session(&suspended).unwrap();
    fx.store.forget_repo(repo.id).unwrap();
    assert_eq!(fx.store.repo(repo.id).unwrap(), None);
    assert!(fx.store.repos().unwrap().is_empty());
    assert_eq!(fx.store.session(&active.id).unwrap(), None);
}

#[test]
fn forgetting_an_unknown_repo_is_an_error() {
    let mut fx = Fixture::new();
    let repo = fx.register("r");
    fx.store.forget_repo(repo.id).unwrap();
    assert!(matches!(
        fx.store.forget_repo(repo.id),
        Err(StoreError::UnknownRepo)
    ));
}

fn paths(store: &Store) -> Vec<std::path::PathBuf> {
    store
        .repos()
        .unwrap()
        .into_iter()
        .map(|repo| repo.path)
        .collect()
}
