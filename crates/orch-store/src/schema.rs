use rusqlite::Connection;

use crate::StoreError;

const MIGRATIONS: &[&str] = &[
    "
CREATE TABLE repos (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    last_used INTEGER NOT NULL
);
CREATE TABLE sessions (
    seq INTEGER PRIMARY KEY,
    id TEXT NOT NULL UNIQUE,
    repo_id INTEGER NOT NULL REFERENCES repos (id) ON DELETE CASCADE,
    slug TEXT NOT NULL,
    branch TEXT NOT NULL,
    base TEXT NOT NULL,
    worktree TEXT NOT NULL,
    phase TEXT NOT NULL,
    preset TEXT NOT NULL,
    last_mode TEXT,
    unseen INTEGER NOT NULL DEFAULT 0,
    stalled INTEGER NOT NULL DEFAULT 0,
    needs_rebase INTEGER NOT NULL DEFAULT 0,
    recovered INTEGER NOT NULL DEFAULT 0,
    worktree_missing INTEGER NOT NULL DEFAULT 0,
    base_missing INTEGER NOT NULL DEFAULT 0,
    muted INTEGER NOT NULL DEFAULT 0,
    pr_number INTEGER,
    pr_checks TEXT,
    pr_review TEXT,
    pr_new_comments INTEGER,
    pr_state TEXT,
    port_base INTEGER,
    port_size INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE conversations (
    session_id TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    conversation_id TEXT NOT NULL,
    PRIMARY KEY (session_id, position)
);
CREATE TABLE trust (
    repo_id INTEGER PRIMARY KEY REFERENCES repos (id) ON DELETE CASCADE,
    hash TEXT NOT NULL,
    approved_at INTEGER NOT NULL
);
CREATE TABLE usage (
    seq INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    conversation_id TEXT NOT NULL,
    segment INTEGER NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cost_usd REAL NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (session_id, conversation_id, segment)
);
CREATE TABLE usage_daily (
    repo_path TEXT NOT NULL,
    day TEXT NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cost_usd REAL NOT NULL,
    PRIMARY KEY (repo_path, day)
);
",
    "
ALTER TABLE repos ADD COLUMN head_branch TEXT;
",
];

pub(crate) fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    let applied: usize = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if applied > MIGRATIONS.len() {
        return Err(StoreError::NewerSchema {
            found: applied,
            supported: MIGRATIONS.len(),
        });
    }
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(applied) {
        let tx = conn.transaction()?;
        tx.execute_batch(migration)?;
        tx.pragma_update(None, "user_version", index + 1)?;
        tx.commit()?;
    }
    Ok(())
}
