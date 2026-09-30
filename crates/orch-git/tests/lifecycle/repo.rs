use orch_git::{Error, GitVersion, Repo};

use crate::common::{Fixture, git};

#[test]
fn opening_a_subdirectory_resolves_the_main_checkout() {
    let fixture = Fixture::new();
    std::fs::create_dir(fixture.root.join("sub")).unwrap();
    let repo = Repo::open(&fixture.root.join("sub")).unwrap();
    assert_eq!(repo.root(), fixture.root);
}

#[test]
fn opening_a_linked_worktree_resolves_the_main_checkout() {
    let fixture = Fixture::new();
    let linked = fixture.outside().join("linked");
    git(
        &fixture.root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "other",
            linked.to_str().unwrap(),
        ],
    );
    let repo = Repo::open(&linked).unwrap();
    assert_eq!(repo.root(), fixture.root);
}

#[test]
fn opening_a_directory_outside_git_fails() {
    let fixture = Fixture::new();
    assert!(Repo::open(fixture.outside()).is_err());
}

#[test]
fn configured_base_branch_wins() {
    let fixture = Fixture::with_origin("trunk");
    assert_eq!(
        fixture.repo().default_base(Some("develop")).unwrap(),
        "develop"
    );
}

#[test]
fn base_branch_falls_back_to_origin_head() {
    let fixture = Fixture::with_origin("trunk");
    assert_eq!(fixture.repo().default_base(None).unwrap(), "trunk");
}

#[test]
fn base_branch_follows_a_default_branch_changed_on_origin_after_cloning() {
    let fixture = Fixture::with_origin("trunk");
    git(
        &fixture.origin,
        &["symbolic-ref", "HEAD", "refs/heads/main"],
    );
    assert_eq!(fixture.repo().default_base(None).unwrap(), "main");
}

#[test]
fn an_unreachable_origin_falls_back_to_the_last_known_origin_head() {
    let fixture = Fixture::with_origin("trunk");
    std::fs::remove_dir_all(&fixture.origin).unwrap();
    assert_eq!(fixture.repo().default_base(None).unwrap(), "trunk");
}

#[test]
fn origin_head_without_a_local_branch_creates_it_untracked_from_origin() {
    let fixture = Fixture::with_origin("trunk");
    let remote_tip = git(&fixture.root, &["rev-parse", "origin/trunk"]);
    git(&fixture.root, &["branch", "-D", "trunk"]);
    let repo = fixture.repo();

    assert_eq!(repo.default_base(None).unwrap(), "trunk");

    assert!(repo.branch_exists("trunk"));
    assert_eq!(git(&fixture.root, &["rev-parse", "trunk"]), remote_tip);
    let config = git(&fixture.root, &["config", "--list", "--local"]);
    assert!(!config.contains("branch.trunk."), "{config}");
}

#[test]
fn origin_head_naming_a_missing_remote_branch_falls_through_to_the_main_checkout() {
    let fixture = Fixture::with_origin("trunk");
    git(
        &fixture.root,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/ghost",
        ],
    );
    std::fs::remove_dir_all(&fixture.origin).unwrap();
    assert_eq!(fixture.repo().default_base(None).unwrap(), "main");
}

#[test]
fn origin_head_resolves_even_with_a_detached_main_checkout() {
    let fixture = Fixture::with_origin("trunk");
    git(&fixture.root, &["switch", "-q", "--detach"]);
    assert_eq!(fixture.repo().default_base(None).unwrap(), "trunk");
}

#[test]
fn base_branch_falls_back_to_the_main_checkout_head_without_origin() {
    let fixture = Fixture::new();
    git(&fixture.root, &["switch", "-q", "-c", "develop"]);
    assert_eq!(fixture.repo().default_base(None).unwrap(), "develop");
}

#[test]
fn base_branch_cannot_be_resolved_from_a_detached_main_checkout() {
    let fixture = Fixture::new();
    git(&fixture.root, &["switch", "-q", "--detach"]);
    assert!(fixture.repo().default_base(None).is_err());
}

#[test]
fn branch_existence_is_reported() {
    let fixture = Fixture::new();
    let repo = fixture.repo();
    assert!(repo.branch_exists("main"));
    assert!(!repo.branch_exists("nope"));
}

#[test]
fn repo_existence_is_reported() {
    let fixture = Fixture::new();
    assert!(Repo::exists(&fixture.root));
    assert!(!Repo::exists(&fixture.root.join("gone")));
}

#[test]
fn installed_git_is_recent_enough() {
    Fixture::new();
    assert!(GitVersion::installed().unwrap() >= GitVersion::MINIMUM);
}

#[test]
fn git_version_is_parsed_from_the_version_banner() {
    assert_eq!(
        GitVersion::parse("git version 2.43.0.windows.1"),
        Some(GitVersion {
            major: 2,
            minor: 43
        })
    );
    assert_eq!(GitVersion::parse("not git"), None);
}

#[test]
fn git_older_than_2_40_is_rejected_with_a_clear_error() {
    let old = GitVersion {
        major: 2,
        minor: 39,
    };
    let error = old.require_minimum().unwrap_err();
    assert_eq!(error, Error::GitTooOld { found: old });
    assert!(error.to_string().contains("2.40"), "{error}");
    assert!(
        GitVersion {
            major: 2,
            minor: 40
        }
        .require_minimum()
        .is_ok()
    );
}

#[test]
fn head_branch_is_the_main_checkouts_current_branch() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.repo().head_branch().unwrap().as_deref(),
        Some("main")
    );
    git(&fixture.root, &["switch", "-q", "-c", "develop"]);
    assert_eq!(
        fixture.repo().head_branch().unwrap().as_deref(),
        Some("develop")
    );
}

#[test]
fn head_branch_of_a_detached_main_checkout_is_none() {
    let fixture = Fixture::new();
    git(&fixture.root, &["switch", "-q", "--detach"]);
    assert_eq!(fixture.repo().head_branch().unwrap(), None);
}
