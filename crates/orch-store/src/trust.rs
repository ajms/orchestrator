use orch_config::TrustHash;
use rusqlite::{OptionalExtension, params};

use crate::time::now_millis;
use crate::{RepoId, Store, StoreError};

impl Store {
    pub fn trust_approval(&self, repo: RepoId) -> Result<Option<TrustHash>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT hash FROM trust WHERE repo_id = ?1",
                params![repo.0],
                |row| row.get::<_, String>(0).map(TrustHash::from_stored),
            )
            .optional()?)
    }

    pub fn approve_trust(&mut self, repo: RepoId, hash: &TrustHash) -> Result<(), StoreError> {
        self.repo(repo)?.ok_or(StoreError::UnknownRepo)?;
        self.conn.execute(
            "INSERT INTO trust (repo_id, hash, approved_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (repo_id) DO UPDATE SET hash = excluded.hash, approved_at = excluded.approved_at",
            params![repo.0, hash.as_str(), now_millis()],
        )?;
        Ok(())
    }
}
