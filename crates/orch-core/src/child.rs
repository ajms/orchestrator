//! Child Sessions: what a Session's Agent asks of the Daemon through
//! `orch session`, and what it is told back.

use serde::{Deserialize, Serialize};

use crate::TaskId;

/// How deep Child Sessions may nest by default: a child of a child.
pub const DEFAULT_MAX_CHILD_DEPTH: usize = 2;
/// Longest prompt a spawn may carry.
pub const MAX_CHILD_PROMPT: usize = 64 * 1024;
/// Longest summary `orch session done` stores.
pub const MAX_CHILD_SUMMARY: usize = 16 * 1024;
/// Most ids one `wait` or `status` names.
pub const MAX_CHILD_IDS: usize = 64;

/// One `orch session` call. Its input is untrusted: it comes from inside the
/// Sandbox, and the Daemon checks that every id names the caller or one of its
/// descendants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SessionOp {
    Spawn {
        prompt: String,
        #[serde(default)]
        task: Option<TaskId>,
        #[serde(default)]
        agent: Option<String>,
        #[serde(default)]
        preset: Option<String>,
    },
    Done {
        summary: String,
    },
    Wait {
        ids: Vec<String>,
    },
    Status {
        #[serde(default)]
        ids: Vec<String>,
    },
    Land {
        id: String,
    },
    Discard {
        id: String,
    },
}

/// Where a Child Session stands, as its parent sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildState {
    /// Its Agent is still at it (or waiting for input).
    Working,
    /// Its Agent said `orch session done`.
    Done,
    /// Landing it onto the parent's Branch failed; it stays open.
    LandFailed,
    /// Setup or its Agent failed before it said done.
    Failed,
    /// It no longer exists: Landed or Discarded.
    Ended,
}

impl ChildState {
    pub fn name(self) -> &'static str {
        match self {
            ChildState::Working => "working",
            ChildState::Done => "done",
            ChildState::LandFailed => "land failed",
            ChildState::Failed => "failed",
            ChildState::Ended => "ended",
        }
    }

    /// `wait` returns once every child is settled.
    pub fn is_settled(self) -> bool {
        !matches!(self, ChildState::Working)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildReport {
    pub id: String,
    #[serde(default)]
    pub parent: Option<String>,
    pub branch: String,
    pub state: ChildState,
    #[serde(default)]
    pub summary: Option<String>,
    /// Why it failed, or why its Landing failed.
    #[serde(default)]
    pub problem: Option<String>,
}

/// Bounds untrusted text by characters, never splitting one.
pub fn checked_text(text: &str, max: usize, what: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(format!("the {what} is empty"));
    }
    if text.len() > max {
        return Err(format!("the {what} is longer than {max} bytes"));
    }
    Ok(text.to_string())
}
