use std::path::PathBuf;

use orch_core::{ConversationId, SessionId, UsageSample};
use rusqlite::{OptionalExtension, Row, Transaction, params};

use crate::time::now_millis;
use crate::{Store, StoreError};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UsageTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

impl UsageTotals {
    fn from_sample(sample: &UsageSample, fallback: UsageTotals) -> Self {
        Self {
            input_tokens: sample.input_tokens.unwrap_or(fallback.input_tokens),
            output_tokens: sample.output_tokens.unwrap_or(fallback.output_tokens),
            cost_usd: sample.cost_usd.unwrap_or(fallback.cost_usd),
        }
    }

    fn below(self, other: UsageTotals) -> bool {
        self.input_tokens < other.input_tokens
            || self.output_tokens < other.output_tokens
            || self.cost_usd < other.cost_usd
    }

    pub fn plus(self, other: UsageTotals) -> Self {
        Self {
            input_tokens: self.input_tokens + other.input_tokens,
            output_tokens: self.output_tokens + other.output_tokens,
            cost_usd: self.cost_usd + other.cost_usd,
        }
    }

    fn minus(self, other: UsageTotals) -> Self {
        Self {
            input_tokens: self.input_tokens - other.input_tokens,
            output_tokens: self.output_tokens - other.output_tokens,
            cost_usd: self.cost_usd - other.cost_usd,
        }
    }

    fn from_columns(row: &Row, first: usize) -> rusqlite::Result<Self> {
        Ok(Self {
            input_tokens: row.get::<_, i64>(first)? as u64,
            output_tokens: row.get::<_, i64>(first + 1)? as u64,
            cost_usd: row.get(first + 2)?,
        })
    }
}

impl std::iter::Sum for UsageTotals {
    fn sum<I: Iterator<Item = Self>>(totals: I) -> Self {
        totals.fold(Self::default(), Self::plus)
    }
}

impl Store {
    pub fn record_usage(
        &mut self,
        session: &SessionId,
        sample: &UsageSample,
    ) -> Result<(), StoreError> {
        let record = self.existing_session(session)?;
        let repo_path = self.repo(record.repo)?.ok_or(StoreError::UnknownRepo)?.path;
        let conversation = sample
            .conversation
            .as_ref()
            .or(record.latest_conversation())
            .ok_or(StoreError::NoConversation)?;
        let now = now_millis();
        let tx = self.conn.transaction()?;
        let latest: Option<(i64, UsageTotals)> = tx
            .query_row(
                "SELECT segment, input_tokens, output_tokens, cost_usd FROM usage
                 WHERE session_id = ?1 AND conversation_id = ?2
                 ORDER BY segment DESC LIMIT 1",
                params![session.as_str(), conversation.as_str()],
                |row| Ok((row.get(0)?, UsageTotals::from_columns(row, 1)?)),
            )
            .optional()?;
        let (segment, totals, added) = match latest {
            Some((segment, stored)) => {
                let totals = UsageTotals::from_sample(sample, stored);
                match totals.below(stored) {
                    false => (segment, totals, totals.minus(stored)),
                    true => {
                        let restarted = UsageTotals::from_sample(sample, UsageTotals::default());
                        (segment + 1, restarted, restarted)
                    }
                }
            }
            None => {
                let totals = UsageTotals::from_sample(sample, UsageTotals::default());
                (0, totals, totals)
            }
        };
        tx.execute(
            "INSERT INTO usage (session_id, conversation_id, segment,
                input_tokens, output_tokens, cost_usd, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (session_id, conversation_id, segment) DO UPDATE SET
                input_tokens = ?4, output_tokens = ?5, cost_usd = ?6, updated_at = ?7",
            params![
                session.as_str(),
                conversation.as_str(),
                segment,
                totals.input_tokens as i64,
                totals.output_tokens as i64,
                totals.cost_usd,
                now,
            ],
        )?;
        add_daily(&tx, &repo_path, now, added)?;
        tx.commit()?;
        Ok(())
    }

    pub fn usage_segments(
        &self,
        session: &SessionId,
    ) -> Result<Vec<(ConversationId, UsageTotals)>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT conversation_id, input_tokens, output_tokens, cost_usd FROM usage
             WHERE session_id = ?1 ORDER BY seq",
        )?;
        let segments = statement
            .query_map(params![session.as_str()], |row| {
                Ok((
                    ConversationId(row.get(0)?),
                    UsageTotals::from_columns(row, 1)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(segments)
    }

    pub fn session_usage(&self, session: &SessionId) -> Result<UsageTotals, StoreError> {
        Ok(self
            .usage_segments(session)?
            .into_iter()
            .map(|(_, segment)| segment)
            .sum())
    }

    pub fn usage_per_repo(&self) -> Result<Vec<(PathBuf, UsageTotals)>, StoreError> {
        self.daily_usage("1", params![])
    }

    pub fn usage_per_repo_today(&self) -> Result<Vec<(PathBuf, UsageTotals)>, StoreError> {
        self.daily_usage("day = date('now', 'localtime')", params![])
    }

    fn daily_usage(
        &self,
        filter: &str,
        args: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<(PathBuf, UsageTotals)>, StoreError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT repo_path, SUM(input_tokens), SUM(output_tokens), SUM(cost_usd)
             FROM usage_daily WHERE {filter} GROUP BY repo_path ORDER BY repo_path"
        ))?;
        let totals = statement
            .query_map(args, |row| {
                Ok((
                    PathBuf::from(row.get::<_, String>(0)?),
                    UsageTotals::from_columns(row, 1)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(totals)
    }
}

fn add_daily(
    tx: &Transaction,
    repo_path: &std::path::Path,
    now: i64,
    added: UsageTotals,
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO usage_daily (repo_path, day, input_tokens, output_tokens, cost_usd)
         VALUES (?1, date(?2, 'unixepoch', 'localtime'), ?3, ?4, ?5)
         ON CONFLICT (repo_path, day) DO UPDATE SET
            input_tokens = input_tokens + ?3,
            output_tokens = output_tokens + ?4,
            cost_usd = cost_usd + ?5",
        params![
            repo_path.to_string_lossy(),
            now / 1000,
            added.input_tokens as i64,
            added.output_tokens as i64,
            added.cost_usd,
        ],
    )?;
    Ok(())
}

pub(crate) fn move_usage(tx: &Transaction, from: &str, to: &str) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO usage_daily (repo_path, day, input_tokens, output_tokens, cost_usd)
         SELECT ?2, day, input_tokens, output_tokens, cost_usd FROM usage_daily
         WHERE repo_path = ?1
         ON CONFLICT (repo_path, day) DO UPDATE SET
            input_tokens = input_tokens + excluded.input_tokens,
            output_tokens = output_tokens + excluded.output_tokens,
            cost_usd = cost_usd + excluded.cost_usd",
        params![from, to],
    )?;
    tx.execute(
        "DELETE FROM usage_daily WHERE repo_path = ?1",
        params![from],
    )?;
    Ok(())
}
