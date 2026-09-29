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

impl Problem {
    pub fn describe(&self, slug: impl Fn(&SessionId) -> String) -> String {
        match self {
            Self::RepoMissing => "Repo missing".into(),
            Self::WorktreeMissing { session } => format!("Worktree missing: {}", slug(session)),
            Self::BaseMissing { session, base } => {
                format!("Base {base} missing: {}", slug(session))
            }
            Self::Leftover { leftover } => format!("Leftover {}", leftover.label()),
            Self::CleanupFailed { session, message } => {
                format!("Cleanup failed for {}: {message}", slug(session))
            }
            Self::UnknownHolder {
                session,
                holder_pid,
                ..
            } => format!("Unknown Holder {} (pid {holder_pid})", session.as_str()),
            Self::PortBlockClash { session, other } => {
                format!("Port block clash: {} and {}", slug(session), slug(other))
            }
        }
    }
}

impl LeftoverView {
    pub fn label(&self) -> String {
        match self {
            Self::Worktree { path } => format!("Worktree {}", path.display()),
            Self::Branch { branch } => format!("Branch {branch}"),
        }
    }
}

impl Fix {
    pub fn label(&self) -> String {
        match self {
            Self::RecreateWorktree { .. } => "Recreate the Worktree".into(),
            Self::DiscardRecord { .. } => "Discard the Session record".into(),
            Self::Retarget { base, .. } => format!("Retarget onto {base} (pick a Branch)"),
            Self::ForgetRepo { .. } => "Forget the Repo".into(),
            Self::AdoptLeftover { .. } => "Adopt as a Session".into(),
            Self::RemoveLeftover { .. } => "Remove (shows what is lost first)".into(),
            Self::ShutdownUnknownHolder { .. } => "Shut the Holder down".into(),
            Self::ReassignPortBlock { .. } => "Reassign the Port block".into(),
        }
    }
}
