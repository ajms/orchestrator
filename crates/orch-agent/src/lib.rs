mod agent;
mod built_in;
mod claude;
mod guard;
mod preset;
mod shell;

pub use agent::{
    AgentAdapter, Argv, Capabilities, LaunchSpec, PayloadError, SubagentTranscripts, TitleWatch,
    TranscriptRead, TranscriptReader,
};
pub use built_in::{Adapter, built_in_names, by_name, with_binary};
pub use claude::{ClaudeCode, mode_from_name, mode_name};
pub use guard::{GuardAnswer, GuardContext, GuardDecision, GuardHit, GuardKind, evaluate_guard};
pub use preset::{INHERIT, Preset, PresetSelection, Presets, ReservedPresetName, UnknownPreset};
