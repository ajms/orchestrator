mod common;

use common::{Fixture, new_session};
use orch_core::{
    ChecksState, ConversationId, Flags, PermissionMode, Phase, PrState, PrStatus, ReviewDecision,
    SessionId,
};
use orch_store::StoreError;

#[test]
fn a_new_session_starts_without_flags_conversations_or_port_block() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let created = fx.session(&repo, "fix-login");

    assert_eq!(created.id, SessionId("session-fix-login".into()));
    assert_eq!(created.repo, repo.id);
    assert_eq!(created.slug, "fix-login");
    assert_eq!(created.branch, "orch/fix-login");
    assert_eq!(created.base, "main");
    assert_eq!(
        created.worktree,
        repo.path.join(".orchestrator/worktrees/fix-login")
    );
    assert_eq!(created.phase, Phase::SettingUp);
    assert_eq!(created.flags, Flags::default());
    assert_eq!(created.preset, "inherit");
    assert_eq!(created.last_mode, None);
    assert!(created.conversations.is_empty());
    assert_eq!(created.latest_conversation(), None);
    assert_eq!(created.port_block, None);
    assert_eq!(created.pr_number(), None);
    assert_eq!(created.created_at, created.updated_at);

    assert_eq!(fx.store.session(&created.id).unwrap(), Some(created));
}

#[test]
fn saved_changes_persist_across_reopening() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let mut session = fx.session(&repo, "feature");

    session.phase = Phase::PrOpen;
    session.base = "orch/other".into();
    session.preset = "edits".into();
    session.last_mode = Some(PermissionMode::AcceptEdits);
    session.flags = Flags {
        unseen: true,
        stalled: true,
        needs_rebase: true,
        pr: Some(PrStatus {
            number: 42,
            checks: ChecksState::Failing,
            review: ReviewDecision::ChangesRequested,
            new_comments: 3,
            state: PrState::Open,
        }),
        recovered: true,
        worktree_missing: true,
        base_missing: true,
        muted: true,
    };
    let saved = fx.store.save_session(&session).unwrap();
    assert!(saved.updated_at >= session.updated_at);
    assert_eq!(saved.created_at, session.created_at);

    fx.reopen();
    let loaded = fx.store.session(&session.id).unwrap().unwrap();
    assert_eq!(loaded, saved);
    assert_eq!(loaded.pr_number(), Some(42));
    assert_eq!(loaded.flags, session.flags);
}

#[test]
fn every_phase_and_mode_round_trips() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let mut session = fx.session(&repo, "s");
    let modes = [
        None,
        Some(PermissionMode::Default),
        Some(PermissionMode::AcceptEdits),
        Some(PermissionMode::Plan),
        Some(PermissionMode::Auto),
        Some(PermissionMode::DontAsk),
        Some(PermissionMode::BypassPermissions),
        None,
    ];
    let phases = [
        Phase::SettingUp,
        Phase::SetupFailed,
        Phase::Active,
        Phase::PrOpen,
        Phase::Suspended,
        Phase::Landed,
        Phase::Discarded,
        Phase::Active,
    ];
    for (phase, mode) in phases.into_iter().zip(modes) {
        session.phase = phase;
        session.last_mode = mode;
        fx.store.save_session(&session).unwrap();
        let loaded = fx.store.session(&session.id).unwrap().unwrap();
        assert_eq!((loaded.phase, loaded.last_mode), (phase, mode));
    }
}

#[test]
fn conversations_are_kept_in_order_and_the_latest_is_resumed() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let session = fx.session(&repo, "s");
    for id in ["c1", "c1", "c2", "c3"] {
        fx.store
            .record_conversation(&session.id, ConversationId(id.into()))
            .unwrap();
    }
    fx.reopen();
    let loaded = fx.store.session(&session.id).unwrap().unwrap();
    assert_eq!(
        loaded.conversations,
        vec![
            ConversationId("c1".into()),
            ConversationId("c2".into()),
            ConversationId("c3".into())
        ]
    );
    assert_eq!(
        loaded.latest_conversation(),
        Some(&ConversationId("c3".into()))
    );
}

#[test]
fn sessions_are_listed_overall_and_per_repo() {
    let mut fx = Fixture::new();
    let a = fx.register("a");
    let b = fx.register("b");
    let a1 = fx.session(&a, "a1");
    let b1 = fx.session(&b, "b1");
    let a2 = fx.session(&a, "a2");

    let ids = |sessions: Vec<orch_store::SessionRecord>| {
        sessions.into_iter().map(|s| s.id).collect::<Vec<_>>()
    };
    assert_eq!(
        ids(fx.store.sessions().unwrap()),
        vec![a1.id.clone(), b1.id.clone(), a2.id.clone()]
    );
    assert_eq!(ids(fx.store.sessions_in(a.id).unwrap()), vec![a1.id, a2.id]);
}

#[test]
fn session_ids_are_unique_and_unknown_sessions_are_reported() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    fx.session(&repo, "s");
    assert!(fx.store.create_session(new_session(&repo, "s")).is_err());

    let ghost = SessionId("ghost".into());
    assert_eq!(fx.store.session(&ghost).unwrap(), None);
    assert!(matches!(
        fx.store
            .record_conversation(&ghost, ConversationId("c".into())),
        Err(StoreError::UnknownSession)
    ));
}
