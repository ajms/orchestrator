use std::path::PathBuf;

use orch_core::SessionId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconcileReport {
    pub repos: Vec<RepoReport>,
    pub unknown_holders: Vec<Finding>,
    pub repaired: Vec<Repair>,
}

impl ReconcileReport {
    pub fn repo(&self, repo: &std::path::Path) -> Option<&RepoReport> {
        self.repos.iter().find(|report| report.repo == repo)
    }

    pub fn findings(&self) -> impl Iterator<Item = &Finding> {
        self.repos
            .iter()
            .flat_map(|report| &report.findings)
            .chain(&self.unknown_holders)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoReport {
    pub repo: PathBuf,
    pub missing: bool,
    pub findings: Vec<Finding>,
}

impl RepoReport {
    pub fn leftovers(&self) -> impl Iterator<Item = &LeftoverView> {
        self.findings
            .iter()
            .filter_map(|finding| match &finding.problem {
                Problem::Leftover { leftover } => Some(leftover),
                _ => None,
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub problem: Problem,
    pub fixes: Vec<Fix>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "problem", rename_all = "snake_case")]
pub enum Problem {
    RepoMissing,
    WorktreeMissing {
        session: SessionId,
    },
    BaseMissing {
        session: SessionId,
        base: String,
    },
    Leftover {
        leftover: LeftoverView,
    },
    CleanupFailed {
        session: SessionId,
        message: String,
    },
    UnknownHolder {
        session: SessionId,
        holder_pid: u32,
        cwd: Option<PathBuf>,
    },
    PortBlockClash {
        session: SessionId,
        other: SessionId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LeftoverView {
    Worktree { path: PathBuf },
    Branch { branch: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "fix", rename_all = "snake_case")]
pub enum Fix {
    RecreateWorktree {
        session: SessionId,
    },
    DiscardRecord {
        session: SessionId,
    },
    Retarget {
        session: SessionId,
        base: String,
    },
    ForgetRepo {
        repo: PathBuf,
    },
    AdoptLeftover {
        repo: PathBuf,
        leftover: LeftoverView,
    },
    RemoveLeftover {
        repo: PathBuf,
        leftover: LeftoverView,
    },
    ShutdownUnknownHolder {
        session: SessionId,
    },
    ReassignPortBlock {
        session: SessionId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "repair", rename_all = "snake_case")]
pub enum Repair {
    AdoptedHolder { session: SessionId },
    SuspendedSession { session: SessionId },
    FinishedCleanup { session: SessionId },
}
