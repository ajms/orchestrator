use std::path::PathBuf;

use orch_core::{AgentState, Phase, SessionId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionView {
    pub id: SessionId,
    pub repo: PathBuf,
    pub slug: String,
    #[serde(default)]
    pub title: Option<String>,
    pub branch: String,
    pub base: String,
    pub worktree: PathBuf,
    pub phase: PhaseView,
    pub agent: Option<AgentStateView>,
    pub flags: FlagsView,
    pub preset: String,
    pub mode: Option<String>,
    pub conversation: Option<String>,
    pub port_base: Option<u16>,
    pub port_size: Option<u16>,
    pub setup_output: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub holder_pid: Option<u32>,
    pub guards_enabled: bool,
    pub guard_prompts: Vec<GuardPrompt>,
    pub context_used_percent: Option<f64>,
    pub cost_usd: Option<f64>,
    pub subagents: Vec<SubagentView>,
}

impl SessionView {
    pub fn title_or_slug(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.slug)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseView {
    SettingUp,
    SetupFailed,
    Active,
    PrOpen,
    Suspended,
    Landed,
    Discarded,
}

impl PhaseView {
    pub fn is_live(self) -> bool {
        matches!(self, Self::Active | Self::PrOpen)
    }
}

impl From<Phase> for PhaseView {
    fn from(phase: Phase) -> Self {
        match phase {
            Phase::SettingUp => Self::SettingUp,
            Phase::SetupFailed => Self::SetupFailed,
            Phase::Active => Self::Active,
            Phase::PrOpen => Self::PrOpen,
            Phase::Suspended => Self::Suspended,
            Phase::Landed => Self::Landed,
            Phase::Discarded => Self::Discarded,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStateView {
    Starting,
    Working,
    NeedsInput,
    Idle,
    Errored,
    Exited,
    Unknown,
}

impl From<AgentState> for AgentStateView {
    fn from(state: AgentState) -> Self {
        match state {
            AgentState::Starting => Self::Starting,
            AgentState::Working => Self::Working,
            AgentState::NeedsInput => Self::NeedsInput,
            AgentState::Idle => Self::Idle,
            AgentState::Errored => Self::Errored,
            AgentState::Exited => Self::Exited,
            AgentState::Unknown => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlagsView {
    pub unseen: bool,
    pub stalled: bool,
    pub needs_rebase: bool,
    pub recovered: bool,
    pub worktree_missing: bool,
    pub base_missing: bool,
    pub muted: bool,
    #[serde(default)]
    pub repo_missing: bool,
    pub pr_number: Option<u64>,
    #[serde(default)]
    pub pr: Option<PrView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrView {
    pub checks: PrChecksView,
    pub review: PrReviewView,
    pub new_comments: u32,
    pub closed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrChecksView {
    None,
    Pending,
    Passing,
    Failing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrReviewView {
    None,
    ReviewRequired,
    Approved,
    ChangesRequested,
}

impl From<&orch_core::PrStatus> for PrView {
    fn from(pr: &orch_core::PrStatus) -> Self {
        use orch_core::{ChecksState, PrState, ReviewDecision};
        Self {
            checks: match pr.checks {
                ChecksState::None => PrChecksView::None,
                ChecksState::Pending => PrChecksView::Pending,
                ChecksState::Passing => PrChecksView::Passing,
                ChecksState::Failing => PrChecksView::Failing,
            },
            review: match pr.review {
                ReviewDecision::None => PrReviewView::None,
                ReviewDecision::ReviewRequired => PrReviewView::ReviewRequired,
                ReviewDecision::Approved => PrReviewView::Approved,
                ReviewDecision::ChangesRequested => PrReviewView::ChangesRequested,
            },
            new_comments: pr.new_comments,
            closed: pr.state == PrState::Closed,
        }
    }
}

impl FlagsView {
    pub fn new(flags: &orch_core::Flags, repo_missing: bool) -> Self {
        Self {
            unseen: flags.unseen,
            stalled: flags.stalled,
            needs_rebase: flags.needs_rebase,
            recovered: flags.recovered,
            worktree_missing: flags.worktree_missing,
            base_missing: flags.base_missing,
            muted: flags.muted,
            repo_missing,
            pr_number: flags.pr.as_ref().map(|pr| pr.number),
            pr: flags.pr.as_ref().map(PrView::from),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardPrompt {
    pub id: u64,
    pub tool: String,
    pub kind: GuardKindView,
    pub target: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardKindView {
    BaseBranch,
    OtherRef,
    WorktreeManagement,
    WriteOutsideWorktree,
    ExternalTool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentView {
    pub id: String,
    pub agent_type: String,
    pub description: String,
    pub tool_count: u32,
    pub done: bool,
}
