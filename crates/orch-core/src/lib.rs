mod agent_state;
mod child;
mod event;
mod flags;
mod gate;
mod phase;
mod plan;
mod stacking;
mod status;
mod subagent;
mod transcript;

pub use agent_state::AgentState;
pub use child::{
    ChildReport, ChildState, DEFAULT_MAX_CHILD_DEPTH, MAX_CHILD_IDS, MAX_CHILD_PROMPT,
    MAX_CHILD_SUMMARY, SessionOp, checked_text,
};
pub use event::{
    AgentEvent, ConversationId, FailureKind, InvalidSessionId, Observation, PermissionMode,
    RateLimit, SessionId, SubagentId, UsageSample,
};
pub use flags::{
    Attention, ChecksState, DEFAULT_STALLED_AFTER, Effect, Flags, PrState, PrStatus, ReviewDecision,
};
pub use gate::{DiscardPlan, GateRefusal};
pub use phase::{InvalidTransition, Phase, PhaseEvent};
pub use plan::{
    Attachment, MAX_ATTACHMENT_ID, MAX_ATTACHMENTS, MAX_BLOCKERS, MAX_LINK, MAX_TASKS, MAX_TITLE,
    Plan, PlanOp, Readiness, Task, TaskId, TaskStatus, join_ids, task_marker,
};
pub use stacking::{Retarget, retarget};
pub use status::SessionStatus;
pub use subagent::{
    Subagent, SubagentMerge, SubagentOutcome, SubagentState, SubagentUsage, SubagentWorktree,
};
pub use transcript::TranscriptEntry;
