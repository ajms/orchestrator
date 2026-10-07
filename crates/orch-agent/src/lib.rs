mod agent;
mod antigravity;
mod built_in;
mod claude;
mod guard;
mod hook_event;
mod hookup;
mod lines;
mod preset;
mod shell;

pub use agent::{
    AgentAdapter, Argv, Capabilities, ConversationTree, Draft, DraftInput, LaunchSpec,
    PayloadError, SubagentTranscripts, TitleWatch, TranscriptRead, TranscriptReader,
};
pub use antigravity::Antigravity;
pub use built_in::{Adapter, built_in_names, by_name, with_binary};
pub use claude::{ClaudeCode, mode_from_name, mode_name};
pub use guard::{
    GUARD_WAIT_SECS, GuardAnswer, GuardContext, GuardDecision, GuardHit, GuardKind, GuardOutcome,
    evaluate_guard, guard_outcome,
};
pub use hook_event::tag_hook_event;
pub use hookup::{AgentHookup, FileEdit, HookupError, HookupState, apply as apply_hookup};
pub use preset::{
    EDITS, INHERIT, Preset, PresetSelection, Presets, ReservedPresetName, RuleVerdict, Rules,
    UnknownPreset,
};
