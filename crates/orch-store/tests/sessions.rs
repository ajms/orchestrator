mod common;

use common::{Fixture, new_session};
use orch_core::{
    ChecksState, ConversationId, Flags, PermissionMode, Phase, PrState, PrStatus, ReviewDecision,
    SessionId,
};
use orch_store::{NewSession, StoreError};

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
    session.pr_base = Some("orch/parent".into());
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
    assert_eq!(loaded.pr_base.as_deref(), Some("orch/parent"));
}

#[test]
fn a_queued_prompt_for_the_agent_persists_until_cleared() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let mut session = fx.session(&repo, "stacked");
    assert_eq!(session.queued_prompt, None);

    session.queued_prompt = Some("Please rebase onto main.".into());
    fx.store.save_session(&session).unwrap();
    fx.reopen();
    let loaded = fx.store.session(&session.id).unwrap().unwrap();
    assert_eq!(
        loaded.queued_prompt.as_deref(),
        Some("Please rebase onto main.")
    );

    session.queued_prompt = None;
    fx.store.save_session(&session).unwrap();
    assert_eq!(
        fx.store
            .session(&session.id)
            .unwrap()
            .unwrap()
            .queued_prompt,
        None
    );
}

#[test]
fn a_session_on_an_existing_branch_is_remembered_across_reopening() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let created = fx
        .store
        .create_session(NewSession {
            existing_branch: true,
            ..new_session(&repo, "feature-x")
        })
        .unwrap();
    let plain = fx.session(&repo, "plain");

    fx.reopen();

    let loaded = |id| fx.store.session(id).unwrap().unwrap().existing_branch;
    assert!(loaded(&created.id));
    assert!(!loaded(&plain.id));
}

#[test]
fn a_session_keeps_the_agent_it_was_created_with() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let pi = fx
        .store
        .create_session(NewSession {
            agent: "pi".into(),
            ..new_session(&repo, "on-pi")
        })
        .unwrap();
    let claude = fx.session(&repo, "on-claude");

    fx.reopen();

    let loaded = |id| fx.store.session(id).unwrap().unwrap().agent;
    assert_eq!(loaded(&pi.id), "pi");
    assert_eq!(loaded(&claude.id), "claude");
}

#[test]
fn a_session_stored_before_agents_were_recorded_resolves_to_claude() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let session = fx.session(&repo, "older");
    let conn = rusqlite::Connection::open(fx.db_path()).unwrap();
    conn.execute_batch("ALTER TABLE sessions DROP COLUMN agent; PRAGMA user_version = 11;")
        .unwrap();

    fx.reopen();

    assert_eq!(
        fx.store.session(&session.id).unwrap().unwrap().agent,
        "claude"
    );
}

#[test]
fn a_session_stored_before_existing_branches_loads_as_on_a_branch_orch_created() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let session = fx.session(&repo, "older");
    let conn = rusqlite::Connection::open(fx.db_path()).unwrap();
    conn.execute_batch(
        "ALTER TABLE sessions DROP COLUMN agent; ALTER TABLE sessions DROP COLUMN pr_base; ALTER TABLE sessions DROP COLUMN existing_branch; PRAGMA user_version = 9;",
    )
    .unwrap();

    fx.reopen();

    assert!(
        !fx.store
            .session(&session.id)
            .unwrap()
            .unwrap()
            .existing_branch
    );
}

#[test]
fn the_last_applied_holder_seq_outlives_a_reopen() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let mut session = fx.session(&repo, "remembered");
    assert_eq!(session.holder_seq, 0);

    session.holder_seq = 42;
    fx.store.save_session(&session).unwrap();
    fx.reopen();
    assert_eq!(
        fx.store.session(&session.id).unwrap().unwrap().holder_seq,
        42
    );
}

#[test]
fn a_session_title_outlives_a_reopen_and_can_be_cleared() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let mut session = fx.session(&repo, "fix-login");
    assert_eq!(session.title, None);

    session.title = Some("login redirect".into());
    fx.store.save_session(&session).unwrap();
    fx.reopen();
    let loaded = fx.store.session(&session.id).unwrap().unwrap();
    assert_eq!(loaded.title.as_deref(), Some("login redirect"));
    assert_eq!(loaded.slug, "fix-login");

    session.title = None;
    fx.store.save_session(&session).unwrap();
    assert_eq!(fx.store.session(&session.id).unwrap().unwrap().title, None);
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

#[test]
fn a_new_session_has_guards_on_and_no_agent_state() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let created = fx.session(&repo, "s");
    assert_eq!(created.agent_state, None);
    assert!(created.guards_enabled);
    assert!(created.guard_allowances.is_empty());
}

#[test]
fn agent_state_guard_switch_and_allowances_persist_across_reopening() {
    use orch_agent::{GuardHit, GuardKind};
    use orch_core::AgentState;

    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let mut session = fx.session(&repo, "s");
    let allowances = vec![
        GuardHit {
            kind: GuardKind::WriteOutsideWorktree,
            target: "/etc/hosts".into(),
        },
        GuardHit {
            kind: GuardKind::BaseBranch,
            target: "main".into(),
        },
    ];
    for state in [
        AgentState::Starting,
        AgentState::Working,
        AgentState::NeedsInput,
        AgentState::Idle,
        AgentState::Errored,
        AgentState::Exited,
        AgentState::Unknown,
    ] {
        session.agent_state = Some(state);
        session.guards_enabled = false;
        session.guard_allowances = allowances.clone();
        fx.store.save_session(&session).unwrap();
        fx.reopen();
        let loaded = fx.store.session(&session.id).unwrap().unwrap();
        assert_eq!(loaded.agent_state, Some(state));
        assert!(!loaded.guards_enabled);
        assert_eq!(loaded.guard_allowances, allowances);
    }

    session.guard_allowances.truncate(1);
    session.agent_state = None;
    fx.store.save_session(&session).unwrap();
    let loaded = fx.store.session(&session.id).unwrap().unwrap();
    assert_eq!(loaded.guard_allowances, allowances[..1]);
    assert_eq!(loaded.agent_state, None);
}

#[test]
fn a_deleted_session_is_gone_with_its_conversations_and_port_block() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let session = fx.session(&repo, "s");
    fx.store
        .record_conversation(&session.id, ConversationId("c1".into()))
        .unwrap();
    let range = orch_config::PortRange {
        start: 20000,
        end: 20009,
        block_size: 10,
    };
    fx.store.allocate_port_block(&session.id, &range).unwrap();

    fx.store.delete_session(&session.id).unwrap();
    assert_eq!(fx.store.session(&session.id).unwrap(), None);
    assert!(matches!(
        fx.store.delete_session(&session.id),
        Err(StoreError::UnknownSession)
    ));
    let other = fx.session(&repo, "t");
    assert_eq!(
        fx.store
            .allocate_port_block(&other.id, &range)
            .unwrap()
            .base,
        20000
    );
}
