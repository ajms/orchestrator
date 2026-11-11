use std::fmt;

#[derive(Debug)]
pub enum StoreError {
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    NewerSchema { found: usize, supported: usize },
    UnknownRepo,
    UnknownSession,
    PortsExhausted,
    NoConversation,
    RepoPathTaken,
    LiveSessions { count: usize },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Sqlite(err) => write!(f, "state database: {err}"),
            StoreError::Io(err) => write!(f, "state database: {err}"),
            StoreError::NewerSchema { found, supported } => write!(
                f,
                "state database has schema version {found}, this orch supports up to {supported}"
            ),
            StoreError::UnknownRepo => f.write_str("unknown Repo"),
            StoreError::UnknownSession => f.write_str("unknown Session"),
            StoreError::NoConversation => f.write_str("the Session has no Conversation yet"),
            StoreError::PortsExhausted => f.write_str("every Port block in the range is reserved"),
            StoreError::RepoPathTaken => f.write_str("another Repo is already registered there"),
            StoreError::LiveSessions { count } => {
                write!(f, "the Repo still has {count} live Session(s)")
            }
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> Self {
        StoreError::Sqlite(err)
    }
}

impl From<std::io::Error> for StoreError {
    fn from(err: std::io::Error) -> Self {
        StoreError::Io(err)
    }
}
