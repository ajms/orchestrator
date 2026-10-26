use std::fmt;
use std::path::PathBuf;

use crate::GitVersion;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Git { command: String, stderr: String },
    Io { context: String, message: String },
    GitTooOld { found: GitVersion },
    NotAnOrchestratorWorktree { path: PathBuf },
    InvalidSubagentWorktreeName { name: String },
    BaseAheadOfOrigin { base: String },
    BaseDivergedFromOrigin { base: String },
    BaseNotFastForwarded { base: String, reason: String },
}

impl Error {
    pub(crate) fn io(context: impl Into<String>, error: std::io::Error) -> Self {
        Error::Io {
            context: context.into(),
            message: error.to_string(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Git { command, stderr } => write!(f, "{command} failed: {}", stderr.trim()),
            Error::Io { context, message } => write!(f, "{context}: {message}"),
            Error::GitTooOld { found } => write!(
                f,
                "git {found} is too old; orch needs git {} or newer",
                GitVersion::MINIMUM
            ),
            Error::NotAnOrchestratorWorktree { path } => write!(
                f,
                "{} is not under the Repo's .orchestrator/worktrees directory",
                path.display()
            ),
            Error::InvalidSubagentWorktreeName { name } => {
                write!(f, "{name:?} is not a valid Subagent worktree name")
            }
            Error::BaseAheadOfOrigin { base } => write!(
                f,
                "{base} has commits origin/{base} doesn't; push them before starting a Session"
            ),
            Error::BaseDivergedFromOrigin { base } => write!(
                f,
                "{base} has diverged from origin/{base}; reconcile them before starting a Session"
            ),
            Error::BaseNotFastForwarded { base, reason } => write!(
                f,
                "{base} is behind origin/{base} and can't be fast-forwarded: {reason}"
            ),
        }
    }
}

impl std::error::Error for Error {}
