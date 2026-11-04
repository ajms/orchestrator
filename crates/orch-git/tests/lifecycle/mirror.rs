use orch_git::update_mirror;

use crate::common::*;

#[test]
fn a_missing_mirror_is_cloned_bare_with_heads_tags_and_head() {
    let fixture = Fixture::with_origin("main");
    git(&fixture.root, &["tag", "v1"]);
    git(&fixture.root, &["push", "-q", "origin", "v1"]);
    let mirror = fixture.outside().join("mirrors/libs/core.git");

    update_mirror(&mirror, fixture.origin.to_str().unwrap()).unwrap();

    assert_eq!(git(&mirror, &["rev-parse", "--is-bare-repository"]), "true");
    assert_eq!(rev(&mirror, "main"), rev(&fixture.root, "main"));
    assert_eq!(rev(&mirror, "v1"), rev(&fixture.root, "v1"));
    assert_eq!(git(&mirror, &["symbolic-ref", "HEAD"]), "refs/heads/main");
}

#[test]
fn an_existing_mirror_follows_origin_and_drops_deleted_branches() {
    let fixture = Fixture::with_origin("main");
    git(&fixture.root, &["push", "-q", "origin", "main:gone"]);
    let mirror = fixture.outside().join("mirror.git");
    let url = fixture.origin.to_str().unwrap();
    update_mirror(&mirror, url).unwrap();

    let tip = fixture.push_upstream("main", "new.txt", "new\n");
    git(&fixture.root, &["push", "-q", "origin", ":gone"]);
    update_mirror(&mirror, url).unwrap();

    assert_eq!(rev(&mirror, "main"), tip);
    assert_eq!(git(&mirror, &["branch", "--list", "gone"]), "");
}

#[test]
fn a_mirror_that_cannot_be_fetched_is_an_error_and_leaves_nothing_behind() {
    let fixture = Fixture::new();
    let mirror = fixture.outside().join("mirror.git");
    let missing = fixture.outside().join("nowhere.git");

    assert!(update_mirror(&mirror, missing.to_str().unwrap()).is_err());
    assert!(!mirror.exists());
}

#[test]
fn a_mirror_follows_a_renamed_default_branch() {
    let fixture = Fixture::with_origin("main");
    let mirror = fixture.outside().join("mirror.git");
    let url = fixture.origin.to_str().unwrap();
    update_mirror(&mirror, url).unwrap();

    git(&fixture.root, &["push", "-q", "origin", "main:dev"]);
    git(&fixture.origin, &["symbolic-ref", "HEAD", "refs/heads/dev"]);
    git(&fixture.root, &["push", "-q", "origin", ":main"]);
    update_mirror(&mirror, url).unwrap();

    assert_eq!(git(&mirror, &["symbolic-ref", "HEAD"]), "refs/heads/dev");
}
