use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::StoreError;
use crate::schema::migrate;

pub struct Store {
    pub(crate) conn: Connection,
}

impl Store {
    pub fn default_path() -> Option<PathBuf> {
        Self::default_path_from_vars(orch_config::xdg::process_env)
    }

    pub fn default_path_from_vars(lookup: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
        let base = orch_config::xdg::state_home(lookup)?;
        Some(base.join("orchestrator").join("state.db"))
    }

    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        migrate(&mut conn)?;
        Ok(Self { conn })
    }
}
