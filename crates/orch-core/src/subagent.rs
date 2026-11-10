use std::time::{Duration, Instant};

use crate::SubagentId;

/// What a Subagent is doing, as far as its Agent reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentState {
    Working,
    Done,
    Errored,
}

/// Token use and cost a Subagent reported when it finished.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubagentUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub turns: Option<u32>,
}

/// What became of a Subagent's work when its worktree was cleaned up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentMerge {
    Merged,
    NothingToMerge,
    /// Merging back failed for a reason other than a conflict.
    NotMerged,
    /// Merging back conflicted on these files (empty if none were reported).
    Conflict {
        files: Vec<String>,
    },
    /// The run failed and its work was discarded, or kept on a branch,
    /// without a merge being attempted.
    Abandoned,
}

/// The worktree a Subagent ran in, and whether its work was merged back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubagentWorktree {
    pub path: Option<String>,
    pub branch: Option<String>,
    pub merge: Option<SubagentMerge>,
    pub message: Option<String>,
    pub preserved_branch: Option<String>,
}

/// How a Subagent ended, as its Agent reported it. Every field is optional:
/// Agents report different things, and none of it is trusted.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SubagentOutcome {
    pub errored: bool,
    pub summary: Option<String>,
    pub failure: Option<String>,
    pub model: Option<String>,
    pub usage: Option<SubagentUsage>,
    pub worktree: Option<Box<SubagentWorktree>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Subagent {
    pub id: SubagentId,
    pub agent_type: String,
    pub description: String,
    pub tool_count: u32,
    pub state: SubagentState,
    pub started_at: Option<Instant>,
    pub duration: Option<Duration>,
    pub outcome: SubagentOutcome,
}

impl Subagent {
    pub fn finished(&self) -> bool {
        self.state != SubagentState::Working
    }
}
