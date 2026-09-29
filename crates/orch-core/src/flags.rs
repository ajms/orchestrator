use std::time::Duration;

pub const DEFAULT_STALLED_AFTER: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChecksState {
    None,
    Pending,
    Passing,
    Failing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReviewDecision {
    None,
    ReviewRequired,
    Approved,
    ChangesRequested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrState {
    Open,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrStatus {
    pub number: u64,
    pub checks: ChecksState,
    pub review: ReviewDecision,
    pub new_comments: u32,
    pub state: PrState,
}

impl PrStatus {
    pub fn opened(number: u64) -> Self {
        Self {
            number,
            checks: ChecksState::None,
            review: ReviewDecision::None,
            new_comments: 0,
            state: PrState::Open,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flags {
    pub unseen: bool,
    pub stalled: bool,
    pub needs_rebase: bool,
    pub pr: Option<PrStatus>,
    pub recovered: bool,
    pub worktree_missing: bool,
    pub base_missing: bool,
    pub muted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attention {
    NeedsInput,
    TurnEnded,
    Errored,
    SetupFailed,
    ChecksFailing,
    ChangesRequested,
    PrMerged,
    PrClosed,
}

impl Attention {
    pub(crate) fn marks_unseen(self) -> bool {
        matches!(
            self,
            Attention::NeedsInput
                | Attention::TurnEnded
                | Attention::Errored
                | Attention::SetupFailed
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Effect {
    RecheckRebase,
}
