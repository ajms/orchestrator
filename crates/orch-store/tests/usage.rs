mod common;

use std::path::Path;

use common::{Fixture, new_session};
use orch_core::{ConversationId, SessionId, UsageSample};
use orch_store::{AgentUsage, StoreError, UsageTotals};

fn sample(conversation: Option<&str>, input: u64, output: u64, cost: f64) -> UsageSample {
    UsageSample {
        conversation: conversation.map(|id| ConversationId(id.into())),
        input_tokens: Some(input),
        output_tokens: Some(output),
        cost_usd: Some(cost),
        ..UsageSample::default()
    }
}

fn totals(input_tokens: u64, output_tokens: u64, cost_usd: f64) -> UsageTotals {
    UsageTotals {
        input_tokens,
        output_tokens,
        cost_usd: Some(cost_usd),
    }
}

fn claude(repo: &Path, totals: UsageTotals) -> AgentUsage {
    AgentUsage {
        repo: repo.into(),
        agent: "claude".into(),
        totals,
    }
}

#[test]
fn a_session_without_samples_has_zero_usage() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let session = fx.session(&repo, "s");
    assert_eq!(
        fx.store.session_usage(&session.id).unwrap(),
        UsageTotals::default()
    );
    assert!(fx.store.usage_segments(&session.id).unwrap().is_empty());
}

#[test]
fn usage_is_totalled_across_conversations_without_resetting_after_a_clear() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let session = fx.session(&repo, "s");
    fx.store
        .record_usage(&session.id, &sample(Some("c1"), 100, 10, 0.5))
        .unwrap();
    fx.store
        .record_usage(&session.id, &sample(Some("c1"), 300, 30, 1.5))
        .unwrap();
    fx.store
        .record_usage(&session.id, &sample(Some("c2"), 50, 5, 0.25))
        .unwrap();

    fx.reopen();
    assert_eq!(
        fx.store.session_usage(&session.id).unwrap(),
        totals(350, 35, 1.75)
    );
    assert_eq!(
        fx.store.usage_segments(&session.id).unwrap(),
        vec![
            (ConversationId("c1".into()), totals(300, 30, 1.5)),
            (ConversationId("c2".into()), totals(50, 5, 0.25)),
        ]
    );
}

#[test]
fn a_sample_without_conversation_counts_toward_the_latest_one() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let session = fx.session(&repo, "s");
    assert!(matches!(
        fx.store.record_usage(&session.id, &sample(None, 1, 1, 0.0)),
        Err(StoreError::NoConversation)
    ));

    fx.store
        .record_conversation(&session.id, ConversationId("c7".into()))
        .unwrap();
    fx.store
        .record_usage(&session.id, &sample(None, 9, 3, 0.1))
        .unwrap();
    assert_eq!(
        fx.store.usage_segments(&session.id).unwrap(),
        vec![(ConversationId("c7".into()), totals(9, 3, 0.1))]
    );
}

#[test]
fn fields_missing_from_a_sample_keep_their_previous_value() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let session = fx.session(&repo, "s");
    fx.store
        .record_usage(&session.id, &sample(Some("c1"), 100, 10, 0.5))
        .unwrap();
    let partial = UsageSample {
        conversation: Some(ConversationId("c1".into())),
        cost_usd: Some(0.75),
        ..UsageSample::default()
    };
    fx.store.record_usage(&session.id, &partial).unwrap();
    assert_eq!(
        fx.store.session_usage(&session.id).unwrap(),
        totals(100, 10, 0.75)
    );
}

#[test]
fn usage_for_an_unknown_session_is_an_error() {
    let mut fx = Fixture::new();
    let ghost = SessionId("ghost".into());
    assert!(matches!(
        fx.store.record_usage(&ghost, &sample(Some("c"), 1, 1, 0.0)),
        Err(StoreError::UnknownSession)
    ));
}

#[test]
fn a_restarted_agent_reporting_lower_totals_opens_a_new_segment() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let session = fx.session(&repo, "s");
    fx.store
        .record_usage(&session.id, &sample(Some("c1"), 100, 10, 1.0))
        .unwrap();
    fx.store
        .record_usage(&session.id, &sample(Some("c1"), 20, 2, 0.25))
        .unwrap();
    assert_eq!(
        fx.store.session_usage(&session.id).unwrap(),
        totals(120, 12, 1.25)
    );

    fx.store
        .record_usage(&session.id, &sample(Some("c1"), 50, 5, 0.5))
        .unwrap();
    assert_eq!(
        fx.store.session_usage(&session.id).unwrap(),
        totals(150, 15, 1.5)
    );
    assert_eq!(
        fx.store.usage_segments(&session.id).unwrap(),
        vec![
            (ConversationId("c1".into()), totals(100, 10, 1.0)),
            (ConversationId("c1".into()), totals(50, 5, 0.5)),
        ]
    );
}

#[test]
fn usage_is_totalled_per_repo_overall_and_for_today() {
    let mut fx = Fixture::new();
    let a = fx.register("a");
    let b = fx.register("b");
    let a1 = fx.session(&a, "a1");
    let a2 = fx.session(&a, "a2");
    let b1 = fx.session(&b, "b1");
    fx.store
        .record_usage(&a1.id, &sample(Some("c1"), 100, 10, 0.5))
        .unwrap();
    fx.store
        .record_usage(&a1.id, &sample(Some("c1"), 300, 30, 1.5))
        .unwrap();
    fx.store
        .record_usage(&a2.id, &sample(Some("c9"), 10, 1, 0.25))
        .unwrap();
    fx.store
        .record_usage(&b1.id, &sample(Some("c5"), 7, 7, 2.0))
        .unwrap();

    let expected = vec![
        claude(&a.path, totals(310, 31, 1.75)),
        claude(&b.path, totals(7, 7, 2.0)),
    ];
    assert_eq!(fx.store.usage_per_repo().unwrap(), expected);
    assert_eq!(fx.store.usage_per_repo_today().unwrap(), expected);
}

#[test]
fn a_forgotten_repo_keeps_counting_toward_usage() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let mut session = fx.session(&repo, "s");
    fx.store
        .record_usage(&session.id, &sample(Some("c1"), 100, 10, 0.5))
        .unwrap();
    session.phase = orch_core::Phase::Discarded;
    fx.store.save_session(&session).unwrap();
    fx.store.forget_repo(repo.id).unwrap();

    let expected = vec![claude(&repo.path, totals(100, 10, 0.5))];
    assert_eq!(fx.store.usage_per_repo().unwrap(), expected);
    assert_eq!(fx.store.usage_per_repo_today().unwrap(), expected);
}

#[test]
fn moving_a_repo_carries_its_usage_along() {
    let mut fx = Fixture::new();
    let repo = fx.register("old");
    let session = fx.session(&repo, "s");
    fx.store
        .record_usage(&session.id, &sample(Some("c1"), 1, 1, 0.5))
        .unwrap();
    let new_path = fx.dir.path().join("new");
    std::fs::rename(&repo.path, &new_path).unwrap();
    let moved = fx
        .store
        .move_repo(repo.id, &orch_store::RepoRoot::resolve(&new_path).unwrap())
        .unwrap();
    assert_eq!(
        fx.store.usage_per_repo().unwrap(),
        vec![claude(&moved.path, totals(1, 1, 0.5))]
    );
}

#[test]
fn usage_is_split_per_agent_and_an_unreported_cost_stays_unknown() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let claude_session = fx.session(&repo, "c");
    let mut agy = new_session(&repo, "agy");
    agy.agent = "antigravity".into();
    let agy_session = fx.store.create_session(agy).unwrap();
    fx.store
        .record_usage(&claude_session.id, &sample(Some("c1"), 100, 10, 0.5))
        .unwrap();
    let without_cost = UsageSample {
        conversation: Some(ConversationId("a1".into())),
        input_tokens: Some(40),
        output_tokens: Some(4),
        ..UsageSample::default()
    };
    fx.store
        .record_usage(&agy_session.id, &without_cost)
        .unwrap();
    fx.store
        .record_usage(&agy_session.id, &without_cost)
        .unwrap();

    let unknown_cost = UsageTotals {
        input_tokens: 40,
        output_tokens: 4,
        cost_usd: None,
    };
    assert_eq!(
        fx.store.session_usage(&agy_session.id).unwrap(),
        unknown_cost
    );
    let expected = vec![
        AgentUsage {
            repo: repo.path.clone(),
            agent: "antigravity".into(),
            totals: unknown_cost,
        },
        claude(&repo.path, totals(100, 10, 0.5)),
    ];
    assert_eq!(fx.store.usage_per_repo().unwrap(), expected);
    assert_eq!(fx.store.usage_per_repo_today().unwrap(), expected);
}

#[test]
fn usage_counted_before_agents_were_recorded_belongs_to_claude() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let session = fx.session(&repo, "s");
    fx.store
        .record_usage(&session.id, &sample(Some("c1"), 100, 10, 0.5))
        .unwrap();
    let conn = rusqlite::Connection::open(fx.db_path()).unwrap();
    let version: usize = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    conn.execute_batch(
        "ALTER TABLE usage_daily RENAME TO newer_daily;
         CREATE TABLE usage_daily (
             repo_path TEXT NOT NULL,
             day TEXT NOT NULL,
             input_tokens INTEGER NOT NULL,
             output_tokens INTEGER NOT NULL,
             cost_usd REAL NOT NULL,
             PRIMARY KEY (repo_path, day)
         );
         INSERT INTO usage_daily
             SELECT repo_path, day, input_tokens, output_tokens, cost_usd FROM newer_daily;
         DROP TABLE newer_daily;",
    )
    .unwrap();
    conn.pragma_update(None, "user_version", version - 1)
        .unwrap();
    drop(conn);

    fx.reopen();
    assert_eq!(
        fx.store.usage_per_repo().unwrap(),
        vec![claude(&repo.path, totals(100, 10, 0.5))]
    );
}
