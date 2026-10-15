mod common;

use common::Fixture;
use orch_core::{ConversationId, UsageSample};
use orch_store::UsageTotals;

fn sample(conversation: &str, input: u64, output: u64, cost: f64) -> UsageSample {
    UsageSample {
        conversation: Some(ConversationId(conversation.into())),
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
        cost_usd,
    }
}

fn reshape_as_left_by_the_antigravity_release(fx: &Fixture, antigravity_session: &str) {
    let conn = rusqlite::Connection::open(fx.db_path()).unwrap();
    conn.execute_batch(&format!(
        "
ALTER TABLE sessions DROP COLUMN holder_seq;
ALTER TABLE sessions ADD COLUMN agent TEXT NOT NULL DEFAULT 'claude';
UPDATE sessions SET agent = 'antigravity' WHERE id = '{antigravity_session}';
ALTER TABLE usage RENAME TO usage_old;
CREATE TABLE usage (
    seq INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL,
    segment INTEGER NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cost_usd REAL,
    updated_at INTEGER NOT NULL,
    UNIQUE (session_id, conversation_id, segment)
);
INSERT INTO usage SELECT * FROM usage_old;
DROP TABLE usage_old;
INSERT INTO conversations VALUES ('{antigravity_session}', 0, 'agy-1');
INSERT INTO usage (session_id, conversation_id, segment, input_tokens, output_tokens, cost_usd, updated_at)
    VALUES ('{antigravity_session}', 'agy-1', 0, 7000, 700, NULL, 0);
ALTER TABLE usage_daily RENAME TO usage_daily_old;
CREATE TABLE usage_daily (
    repo_path TEXT NOT NULL,
    agent TEXT NOT NULL,
    day TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cost_usd REAL,
    PRIMARY KEY (repo_path, agent, day)
);
INSERT INTO usage_daily
    SELECT repo_path, 'claude', day, input_tokens, output_tokens, cost_usd FROM usage_daily_old;
INSERT INTO usage_daily
    SELECT repo_path, 'antigravity', day, 7000, 700, NULL FROM usage_daily_old;
DROP TABLE usage_daily_old;
PRAGMA user_version = 7;
"
    ))
    .unwrap();
}

#[test]
fn a_state_database_from_the_antigravity_release_opens_without_its_antigravity_sessions() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let claude = fx.session(&repo, "claude");
    let antigravity = fx.session(&repo, "antigravity");
    fx.store
        .record_usage(&claude.id, &sample("c1", 100, 10, 0.5))
        .unwrap();
    reshape_as_left_by_the_antigravity_release(&fx, &antigravity.id.0);

    fx.reopen();

    let ids: Vec<_> = fx
        .store
        .sessions()
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(ids, vec![claude.id.clone()]);
    assert_eq!(
        fx.store.session_usage(&claude.id).unwrap(),
        totals(100, 10, 0.5)
    );
    assert_eq!(
        fx.store.usage_per_repo().unwrap(),
        vec![(repo.path.clone(), totals(100, 10, 0.5))]
    );
}

#[test]
fn usage_and_sessions_are_written_again_after_leaving_the_antigravity_release() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let claude = fx.session(&repo, "claude");
    let antigravity = fx.session(&repo, "antigravity");
    fx.store
        .record_usage(&claude.id, &sample("c1", 100, 10, 0.5))
        .unwrap();
    reshape_as_left_by_the_antigravity_release(&fx, &antigravity.id.0);
    fx.reopen();

    fx.store
        .record_usage(&claude.id, &sample("c1", 300, 30, 1.5))
        .unwrap();
    let later = fx.session(&repo, "later");

    assert_eq!(
        fx.store.usage_per_repo().unwrap(),
        vec![(repo.path.clone(), totals(300, 30, 1.5))]
    );
    assert_eq!(fx.store.session(&later.id).unwrap(), Some(later));
}

#[test]
fn a_session_allowed_by_a_later_release_still_loads_without_that_allowance() {
    let mut fx = Fixture::new();
    let repo = fx.register("proj");
    let session = fx.session(&repo, "boxed");
    let conn = rusqlite::Connection::open(fx.db_path()).unwrap();
    conn.execute_batch(&format!(
        "INSERT INTO guard_allowances VALUES
            ('{id}', 0, 'push', 'orch/boxed'),
            ('{id}', 1, 'from_a_later_release', 'refs/heads/main'),
            ('{id}', 2, 'pull_request', 'orch/boxed');",
        id = session.id.0
    ))
    .unwrap();

    fx.reopen();

    let loaded = fx.store.session(&session.id).unwrap().unwrap();
    assert_eq!(
        loaded.guard_allowances,
        [
            orch_agent::GuardHit {
                kind: orch_agent::GuardKind::Push,
                target: "orch/boxed".into(),
            },
            orch_agent::GuardHit {
                kind: orch_agent::GuardKind::PullRequest,
                target: "orch/boxed".into(),
            },
        ]
    );
    assert_eq!(fx.store.sessions().unwrap().len(), 1);
}
