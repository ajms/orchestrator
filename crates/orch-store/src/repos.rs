use std::path::{Path, PathBuf};

use rusqlite::{OptionalExtension, Row, params};

use crate::time::now_millis;
use crate::{RepoRoot, Store, StoreError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RepoId(pub i64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub id: RepoId,
    pub path: PathBuf,
    pub missing: bool,
    pub head_branch_at_registration: Option<String>,
}

const COLUMNS: &str = "id, path, head_branch";

fn repo_from(row: &Row) -> rusqlite::Result<Repo> {
    let path = PathBuf::from(row.get::<_, String>(1)?);
    Ok(Repo {
        id: RepoId(row.get(0)?),
        missing: !path.is_dir(),
        head_branch_at_registration: row.get(2)?,
        path,
    })
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

impl Store {
    pub fn register_repo(&mut self, root: &RepoRoot) -> Result<Repo, StoreError> {
        let path = path_text(root.path());
        self.conn.execute(
            "INSERT INTO repos (path, head_branch, last_used)
             VALUES (?1, ?2, (SELECT COALESCE(MAX(last_used), 0) + 1 FROM repos))
             ON CONFLICT (path) DO UPDATE SET last_used = excluded.last_used",
            params![path, root.head_branch()],
        )?;
        self.repo_by_path(root.path())?
            .ok_or(StoreError::UnknownRepo)
    }

    pub fn repos(&self) -> Result<Vec<Repo>, StoreError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM repos ORDER BY last_used DESC"
        ))?;
        let repos = statement
            .query_map([], repo_from)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(repos)
    }

    pub fn repo(&self, id: RepoId) -> Result<Option<Repo>, StoreError> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM repos WHERE id = ?1"),
                params![id.0],
                repo_from,
            )
            .optional()?)
    }

    pub fn repo_by_path(&self, path: &Path) -> Result<Option<Repo>, StoreError> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM repos WHERE path = ?1"),
                params![path_text(path)],
                repo_from,
            )
            .optional()?)
    }

    pub fn move_repo(&mut self, id: RepoId, to: &RepoRoot) -> Result<Repo, StoreError> {
        let from = self.repo(id)?.ok_or(StoreError::UnknownRepo)?.path;
        if self
            .repo_by_path(to.path())?
            .is_some_and(|other| other.id != id)
        {
            return Err(StoreError::RepoPathTaken);
        }
        let relocated: Vec<(String, PathBuf)> = self
            .sessions_in(id)?
            .into_iter()
            .filter_map(|session| {
                let rest = session.worktree.strip_prefix(&from).ok()?;
                Some((session.id.0, to.path().join(rest)))
            })
            .collect();
        let tx = self.conn.transaction()?;
        crate::usage::move_usage(&tx, &path_text(&from), &path_text(to.path()))?;
        tx.execute(
            "UPDATE repos SET path = ?2 WHERE id = ?1",
            params![id.0, path_text(to.path())],
        )?;
        for (session, worktree) in relocated {
            tx.execute(
                "UPDATE sessions SET worktree = ?2, updated_at = ?3 WHERE id = ?1",
                params![session, path_text(&worktree), now_millis()],
            )?;
        }
        tx.commit()?;
        self.repo(id)?.ok_or(StoreError::UnknownRepo)
    }

    pub fn forget_repo(&mut self, id: RepoId) -> Result<(), StoreError> {
        self.repo(id)?.ok_or(StoreError::UnknownRepo)?;
        let count = self
            .sessions_in(id)?
            .iter()
            .filter(|session| !session.phase.is_terminal())
            .count();
        if count > 0 {
            return Err(StoreError::LiveSessions { count });
        }
        self.conn
            .execute("DELETE FROM repos WHERE id = ?1", params![id.0])?;
        Ok(())
    }
}
