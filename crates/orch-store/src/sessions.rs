use std::path::PathBuf;
use std::time::SystemTime;

use orch_agent::GuardHit;
use orch_core::{AgentState, ConversationId, Flags, PermissionMode, Phase, PrStatus, SessionId};
use rusqlite::{OptionalExtension, Row, Transaction, params};

use crate::codec::Stored;
use crate::ports::port_block_from;
use crate::time::{from_millis, millis, now_millis};
use crate::{PortBlock, RepoId, Store, StoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSession {
    pub id: SessionId,
    pub repo: RepoId,
    pub slug: String,
    pub branch: String,
    pub base: String,
    pub worktree: PathBuf,
    pub phase: Phase,
    pub preset: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub id: SessionId,
    pub repo: RepoId,
    pub slug: String,
    pub title: Option<String>,
    pub branch: String,
    pub base: String,
    pub worktree: PathBuf,
    pub phase: Phase,
    pub flags: Flags,
    pub preset: String,
    pub last_mode: Option<PermissionMode>,
    pub agent_state: Option<AgentState>,
    pub guards_enabled: bool,
    pub guard_allowances: Vec<GuardHit>,
    pub conversations: Vec<ConversationId>,
    pub port_block: Option<PortBlock>,
    pub queued_prompt: Option<String>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

impl SessionRecord {
    pub fn latest_conversation(&self) -> Option<&ConversationId> {
        self.conversations.last()
    }

    pub fn pr_number(&self) -> Option<u64> {
        self.flags.pr.as_ref().map(|pr| pr.number)
    }
}

const COLUMNS: &str = "id, repo_id, slug, branch, base, worktree, phase, preset, last_mode,
    unseen, stalled, needs_rebase, recovered, worktree_missing, base_missing, muted,
    pr_number, pr_checks, pr_review, pr_new_comments, pr_state,
    port_base, port_size, created_at, updated_at, agent_state, guards_enabled, queued_prompt, title";

fn session_from(row: &Row) -> rusqlite::Result<SessionRecord> {
    let pr = match row.get::<_, Option<i64>>("pr_number")? {
        None => None,
        Some(number) => Some(PrStatus {
            number: number as u64,
            checks: row.get::<_, Stored<_>>("pr_checks")?.0,
            review: row.get::<_, Stored<_>>("pr_review")?.0,
            new_comments: row.get("pr_new_comments")?,
            state: row.get::<_, Stored<_>>("pr_state")?.0,
        }),
    };
    Ok(SessionRecord {
        id: SessionId(row.get("id")?),
        repo: RepoId(row.get("repo_id")?),
        slug: row.get("slug")?,
        title: row.get("title")?,
        branch: row.get("branch")?,
        base: row.get("base")?,
        worktree: PathBuf::from(row.get::<_, String>("worktree")?),
        phase: row.get::<_, Stored<_>>("phase")?.0,
        flags: Flags {
            unseen: row.get("unseen")?,
            stalled: row.get("stalled")?,
            needs_rebase: row.get("needs_rebase")?,
            pr,
            recovered: row.get("recovered")?,
            worktree_missing: row.get("worktree_missing")?,
            base_missing: row.get("base_missing")?,
            muted: row.get("muted")?,
        },
        preset: row.get("preset")?,
        last_mode: row
            .get::<_, Option<Stored<PermissionMode>>>("last_mode")?
            .map(|mode| mode.0),
        agent_state: row
            .get::<_, Option<Stored<AgentState>>>("agent_state")?
            .map(|state| state.0),
        guards_enabled: row.get("guards_enabled")?,
        guard_allowances: Vec::new(),
        conversations: Vec::new(),
        port_block: port_block_from(row)?,
        queued_prompt: row.get("queued_prompt")?,
        created_at: from_millis(row.get("created_at")?),
        updated_at: from_millis(row.get("updated_at")?),
    })
}

impl Store {
    pub fn create_session(&mut self, new: NewSession) -> Result<SessionRecord, StoreError> {
        let now = now_millis();
        self.conn.execute(
            "INSERT INTO sessions
                (id, repo_id, slug, branch, base, worktree, phase, preset, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                new.id.as_str(),
                new.repo.0,
                new.slug,
                new.branch,
                new.base,
                new.worktree.to_string_lossy(),
                Stored(new.phase),
                new.preset,
                now,
            ],
        )?;
        self.existing_session(&new.id)
    }

    pub fn save_session(&mut self, session: &SessionRecord) -> Result<SessionRecord, StoreError> {
        let flags = &session.flags;
        let pr = flags.pr.as_ref();
        let changed = self.conn.execute(
            "UPDATE sessions SET
                slug = ?2, branch = ?3, base = ?4, worktree = ?5, phase = ?6, preset = ?7,
                last_mode = ?8, unseen = ?9, stalled = ?10, needs_rebase = ?11, recovered = ?12,
                worktree_missing = ?13, base_missing = ?14, muted = ?15, pr_number = ?16,
                pr_checks = ?17, pr_review = ?18, pr_new_comments = ?19, pr_state = ?20,
                updated_at = ?21, agent_state = ?22, guards_enabled = ?23, queued_prompt = ?24,
                title = ?25
             WHERE id = ?1",
            params![
                session.id.as_str(),
                session.slug,
                session.branch,
                session.base,
                session.worktree.to_string_lossy(),
                Stored(session.phase),
                session.preset,
                session.last_mode.map(Stored),
                flags.unseen,
                flags.stalled,
                flags.needs_rebase,
                flags.recovered,
                flags.worktree_missing,
                flags.base_missing,
                flags.muted,
                pr.map(|pr| pr.number as i64),
                pr.map(|pr| Stored(pr.checks)),
                pr.map(|pr| Stored(pr.review)),
                pr.map(|pr| pr.new_comments),
                pr.map(|pr| Stored(pr.state)),
                now_millis().max(millis(session.updated_at)),
                session.agent_state.map(Stored),
                session.guards_enabled,
                session.queued_prompt,
                session.title,
            ],
        )?;
        if changed == 0 {
            return Err(StoreError::UnknownSession);
        }
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM guard_allowances WHERE session_id = ?1",
            params![session.id.as_str()],
        )?;
        for (position, hit) in session.guard_allowances.iter().enumerate() {
            tx.execute(
                "INSERT INTO guard_allowances (session_id, position, kind, target)
                 VALUES (?1, ?2, ?3, ?4)",
                params![session.id.as_str(), position, Stored(hit.kind), hit.target],
            )?;
        }
        tx.commit()?;
        self.existing_session(&session.id)
    }

    pub fn delete_session(&mut self, id: &SessionId) -> Result<(), StoreError> {
        let deleted = self
            .conn
            .execute("DELETE FROM sessions WHERE id = ?1", params![id.as_str()])?;
        match deleted {
            0 => Err(StoreError::UnknownSession),
            _ => Ok(()),
        }
    }

    pub fn record_conversation(
        &mut self,
        session: &SessionId,
        conversation: ConversationId,
    ) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        let known = tx
            .query_row(
                "SELECT 1 FROM sessions WHERE id = ?1",
                params![session.as_str()],
                |_| Ok(()),
            )
            .optional()?;
        if known.is_none() {
            return Err(StoreError::UnknownSession);
        }
        let latest: Option<(i64, String)> = tx
            .query_row(
                "SELECT position, conversation_id FROM conversations
                 WHERE session_id = ?1 ORDER BY position DESC LIMIT 1",
                params![session.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if latest.as_ref().map(|(_, id)| id.as_str()) != Some(conversation.as_str()) {
            let position = latest.map_or(0, |(position, _)| position + 1);
            tx.execute(
                "INSERT INTO conversations (session_id, position, conversation_id)
                 VALUES (?1, ?2, ?3)",
                params![session.as_str(), position, conversation.as_str()],
            )?;
            touch(&tx, session)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn session(&self, id: &SessionId) -> Result<Option<SessionRecord>, StoreError> {
        let found = self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM sessions WHERE id = ?1"),
                params![id.as_str()],
                session_from,
            )
            .optional()?;
        found
            .map(|session| self.with_conversations(session))
            .transpose()
    }

    pub fn sessions(&self) -> Result<Vec<SessionRecord>, StoreError> {
        self.query_sessions(&format!("SELECT {COLUMNS} FROM sessions ORDER BY seq"), &[])
    }

    pub fn sessions_in(&self, repo: RepoId) -> Result<Vec<SessionRecord>, StoreError> {
        self.query_sessions(
            &format!("SELECT {COLUMNS} FROM sessions WHERE repo_id = ?1 ORDER BY seq"),
            &[&repo.0],
        )
    }

    fn query_sessions(
        &self,
        sql: &str,
        args: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<SessionRecord>, StoreError> {
        let mut statement = self.conn.prepare(sql)?;
        let sessions = statement
            .query_map(args, session_from)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        sessions
            .into_iter()
            .map(|session| self.with_conversations(session))
            .collect()
    }

    pub(crate) fn existing_session(&self, id: &SessionId) -> Result<SessionRecord, StoreError> {
        self.session(id)?.ok_or(StoreError::UnknownSession)
    }

    fn with_conversations(&self, mut session: SessionRecord) -> Result<SessionRecord, StoreError> {
        let mut statement = self.conn.prepare_cached(
            "SELECT conversation_id FROM conversations WHERE session_id = ?1 ORDER BY position",
        )?;
        session.conversations = statement
            .query_map(params![session.id.as_str()], |row| {
                row.get(0).map(ConversationId)
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut statement = self.conn.prepare_cached(
            "SELECT kind, target FROM guard_allowances WHERE session_id = ?1 ORDER BY position",
        )?;
        session.guard_allowances = statement
            .query_map(params![session.id.as_str()], |row| {
                Ok(GuardHit {
                    kind: row.get::<_, Stored<_>>(0)?.0,
                    target: row.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(session)
    }
}

pub(crate) fn touch(tx: &Transaction, session: &SessionId) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE sessions SET updated_at = MAX(updated_at, ?2) WHERE id = ?1",
        params![session.as_str(), now_millis()],
    )?;
    Ok(())
}
