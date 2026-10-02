mod common;

use common::Fixture;
use orch_config::TrustHash;

#[test]
fn the_latest_approval_is_kept_across_reopening() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    fx.store
        .approve_trust(repo.id, &TrustHash::from_stored("aaa"))
        .unwrap();
    fx.store
        .approve_trust(repo.id, &TrustHash::from_stored("bbb"))
        .unwrap();
    fx.reopen();
    assert_eq!(
        fx.store.trust_approval(repo.id).unwrap(),
        Some(TrustHash::from_stored("bbb"))
    );
}

#[test]
fn approval_is_per_repo() {
    let mut fx = Fixture::new();
    let a = fx.register("a");
    let b = fx.register("b");
    fx.store
        .approve_trust(a.id, &TrustHash::from_stored("aaa"))
        .unwrap();
    assert_eq!(
        fx.store.trust_approval(a.id).unwrap(),
        Some(TrustHash::from_stored("aaa"))
    );
    assert_eq!(fx.store.trust_approval(b.id).unwrap(), None);
}

#[test]
fn approved_hash_from_the_store_makes_committed_scripts_usable() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    std::fs::write(repo.path.join(".orchestrator.toml"), "setup = \"make\"\n").unwrap();
    let loader = orch_config::ConfigLoader::new(fx.dir.path().join("config.toml"));

    let approval = fx.store.trust_approval(repo.id).unwrap();
    let config = loader.repo(&repo.path, approval.as_ref()).unwrap();
    assert!(config.setup_script().is_err());

    let request = config.trust_request().unwrap();
    fx.store.approve_trust(repo.id, &request.hash).unwrap();
    let approval = fx.store.trust_approval(repo.id).unwrap();
    let config = loader.repo(&repo.path, approval.as_ref()).unwrap();
    assert_eq!(config.setup_script(), Ok(Some("make")));
}
