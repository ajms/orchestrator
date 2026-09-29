mod common;

use common::Fixture;
use orch_config::PortRange;
use orch_core::Phase;
use orch_store::{PortBlock, StoreError};

const RANGE: PortRange = PortRange {
    start: 4000,
    end: 4029,
    block_size: 10,
};

fn block(base: u16) -> PortBlock {
    PortBlock { base, size: 10 }
}

#[test]
fn sessions_get_the_lowest_free_block_and_keep_it() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let a = fx.session(&repo, "a");
    let b = fx.session(&repo, "b");

    assert_eq!(
        fx.store.allocate_port_block(&a.id, &RANGE).unwrap(),
        block(4000)
    );
    assert_eq!(
        fx.store.allocate_port_block(&b.id, &RANGE).unwrap(),
        block(4010)
    );
    assert_eq!(
        fx.store.allocate_port_block(&a.id, &RANGE).unwrap(),
        block(4000)
    );
    assert_eq!(
        fx.store.session(&a.id).unwrap().unwrap().port_block,
        Some(block(4000))
    );
}

#[test]
fn freed_blocks_are_reused_lowest_first() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let a = fx.session(&repo, "a");
    let b = fx.session(&repo, "b");
    let c = fx.session(&repo, "c");
    fx.store.allocate_port_block(&a.id, &RANGE).unwrap();
    fx.store.allocate_port_block(&b.id, &RANGE).unwrap();
    fx.store.free_port_block(&a.id).unwrap();

    assert_eq!(fx.store.session(&a.id).unwrap().unwrap().port_block, None);
    assert_eq!(
        fx.store.allocate_port_block(&c.id, &RANGE).unwrap(),
        block(4000)
    );
}

#[test]
fn allocation_fails_when_every_block_is_reserved() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    for slug in ["a", "b", "c"] {
        let session = fx.session(&repo, slug);
        fx.store.allocate_port_block(&session.id, &RANGE).unwrap();
    }
    let d = fx.session(&repo, "d");
    assert!(matches!(
        fx.store.allocate_port_block(&d.id, &RANGE),
        Err(StoreError::PortsExhausted)
    ));
}

#[test]
fn saving_a_session_never_changes_its_port_block() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let mut session = fx.session(&repo, "a");
    fx.store.allocate_port_block(&session.id, &RANGE).unwrap();
    session.phase = Phase::Suspended;
    let saved = fx.store.save_session(&session).unwrap();
    assert_eq!(saved.port_block, Some(block(4000)));
}

#[test]
fn blocks_reserved_under_an_old_range_are_avoided_after_the_range_changes() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let a = fx.session(&repo, "a");
    let b = fx.session(&repo, "b");
    let shifted = PortRange {
        start: 3995,
        end: 4029,
        block_size: 10,
    };
    fx.store.allocate_port_block(&a.id, &RANGE).unwrap();
    assert_eq!(
        fx.store.allocate_port_block(&b.id, &shifted).unwrap(),
        block(4015)
    );
}

#[test]
fn rebuilding_releases_blocks_of_ended_sessions_and_keeps_suspended_ones() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let mut landed = fx.session(&repo, "landed");
    let mut suspended = fx.session(&repo, "suspended");
    let mut discarded = fx.session(&repo, "discarded");
    for session in [&landed, &suspended, &discarded] {
        fx.store.allocate_port_block(&session.id, &RANGE).unwrap();
    }
    landed.phase = Phase::Landed;
    suspended.phase = Phase::Suspended;
    discarded.phase = Phase::Discarded;
    for session in [&landed, &suspended, &discarded] {
        fx.store.save_session(session).unwrap();
    }

    let reserved = fx.store.rebuild_port_blocks().unwrap();
    assert_eq!(reserved, vec![(suspended.id.clone(), block(4010))]);
    assert_eq!(
        fx.store.session(&landed.id).unwrap().unwrap().port_block,
        None
    );
    assert_eq!(
        fx.store.session(&suspended.id).unwrap().unwrap().port_block,
        Some(block(4010))
    );
}

#[test]
fn a_recovered_session_holds_the_block_it_uses_and_names_who_else_holds_it() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let a = fx.session(&repo, "a");
    let b = fx.session(&repo, "b");
    let c = fx.session(&repo, "c");
    fx.store.allocate_port_block(&a.id, &RANGE).unwrap();

    assert_eq!(
        fx.store.hold_port_block(&b.id, block(4010)).unwrap(),
        vec![]
    );
    assert_eq!(
        fx.store.hold_port_block(&c.id, block(4000)).unwrap(),
        vec![a.id.clone()]
    );
    assert_eq!(
        fx.store.session(&c.id).unwrap().unwrap().port_block,
        Some(block(4000))
    );
    let d = fx.session(&repo, "d");
    assert_eq!(
        fx.store.allocate_port_block(&d.id, &RANGE).unwrap(),
        block(4020)
    );
}
