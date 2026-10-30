mod agent;
mod claude;
mod guard;
mod preset;
mod registry;
mod sandbox;
mod sandbox_failure;
mod shell;

pub use agent::{
    AgentAdapter, Argv, Capabilities, LaunchSpec, PayloadError, SubagentTranscripts, TitleWatch,
    TranscriptRead, TranscriptReader, WorktreeRequest,
};
pub use claude::{CLAUDE, ClaudeCode, mode_from_name, mode_name};
pub use guard::{GuardAnswer, GuardContext, GuardDecision, GuardHit, GuardKind, evaluate_guard};
pub use preset::{INHERIT, Preset, PresetSelection, Presets, ReservedPresetName, UnknownPreset};
pub use registry::{Adapter, adapter_named, implied_agent};
pub use sandbox::{
    BWRAP, Placeholder, SandboxLayout, SandboxNeeds, SandboxPaths, sandbox_args, sandbox_carve_outs,
};
pub use sandbox_failure::{SandboxFailure, sandbox_refusal};
