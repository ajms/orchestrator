use std::path::{Path, PathBuf};

use orch_core::{
    AgentEvent, ConversationId, GuardedAction, PermissionMode, SessionId, SessionStatus,
    SubagentId, TranscriptEntry,
};

use crate::hookup::AgentHookup;
use crate::{GuardAnswer, Preset, RuleVerdict};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub hooks: bool,
    pub resume: bool,
    pub usage: bool,
    pub modes: bool,
    pub guards: bool,
    pub subagents: bool,
    pub titles: bool,
    pub transcripts: bool,
}

impl Capabilities {
    pub fn session_status(&self) -> SessionStatus {
        if self.hooks {
            SessionStatus::new()
        } else {
            SessionStatus::unobserved()
        }
    }

    pub fn guards_available(&self) -> bool {
        self.hooks && self.guards
    }

    pub fn effective_preset(&self, preset: Preset) -> Preset {
        if self.modes {
            preset
        } else {
            Preset::inherit()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Argv {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftInput {
    Instruction,
    InstructionAndBaseDiff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftOutcome {
    Drafted(String),
    NoResult,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub argv: Argv,
    pub input: DraftInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub session: SessionId,
    pub orch_program: String,
    pub preset: Preset,
    pub prompt: Option<String>,
}

impl LaunchSpec {
    pub fn new(session: SessionId, orch_program: impl Into<String>, preset: Preset) -> Self {
        Self {
            session,
            orch_program: orch_program.into(),
            preset,
            prompt: None,
        }
    }

    pub fn resume_mode(&self, observed: Option<PermissionMode>) -> Option<PermissionMode> {
        self.preset.mode.and(observed.or(self.preset.mode))
    }

    pub fn with_prompt(self, prompt: impl Into<String>) -> Self {
        Self {
            prompt: Some(prompt.into()),
            ..self
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadError(pub String);

pub struct RuleScope<'a> {
    pub cwd: Option<&'a Path>,
    pub worktree: &'a Path,
    pub lookup: &'a dyn Fn(&str) -> Option<String>,
}

pub trait TitleWatch: Send {
    fn follow(&mut self, payload: &str);
    fn poll(&mut self) -> Vec<AgentEvent>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscriptRead {
    pub reset: bool,
    pub entries: Vec<TranscriptEntry>,
}

pub trait SubagentTranscripts: Send {
    fn follow(&mut self, payload: &str);
    fn locate(&self, subagent: &SubagentId) -> Option<PathBuf>;
    fn reader(&self) -> Box<dyn TranscriptReader>;
}

pub trait TranscriptReader: Send {
    fn read(&mut self, path: &Path) -> TranscriptRead;
}

pub trait ConversationTree: Send {
    fn restart(&mut self, conversation: Option<&ConversationId>);
    fn hook(&mut self, payload: &str) -> Result<Vec<AgentEvent>, PayloadError>;
    fn tap(&mut self, payload: &str) -> Result<Vec<AgentEvent>, PayloadError>;
}

pub trait AgentAdapter {
    fn capabilities(&self) -> Capabilities;

    fn launch(&self, spec: &LaunchSpec) -> Argv;

    fn modes(&self) -> &'static [PermissionMode] {
        &[]
    }

    fn resume(
        &self,
        _spec: &LaunchSpec,
        _conversation: &ConversationId,
        _mode: Option<PermissionMode>,
    ) -> Option<Argv> {
        None
    }

    fn restart(
        &self,
        spec: &LaunchSpec,
        conversation: Option<&ConversationId>,
        mode: Option<PermissionMode>,
    ) -> Argv {
        conversation
            .and_then(|conversation| self.resume(spec, conversation, mode))
            .unwrap_or_else(|| self.launch(spec))
    }

    fn draft(&self, _conversation: Option<&ConversationId>) -> Option<Draft> {
        None
    }

    fn encode_draft(&self, prompt: &str) -> String {
        prompt.into()
    }

    fn decode_draft(&self, stdout: &str) -> DraftOutcome {
        DraftOutcome::Drafted(stdout.into())
    }

    fn map_hook(&self, _payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
        Ok(Vec::new())
    }

    fn map_tap(&self, _payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
        Ok(Vec::new())
    }

    fn title_watch(&self) -> Option<Box<dyn TitleWatch>> {
        None
    }

    fn subagent_transcripts(&self) -> Option<Box<dyn SubagentTranscripts>> {
        None
    }

    fn conversation_tree(&self) -> Option<Box<dyn ConversationTree>> {
        None
    }

    fn is_guard_payload(&self, _payload: &str) -> bool {
        false
    }

    fn guard_answer(&self, _answer: &GuardAnswer) -> Option<String> {
        None
    }

    fn enforces_rules(&self) -> bool {
        false
    }

    fn rule_verdict(
        &self,
        _preset: &Preset,
        _action: &GuardedAction,
        _scope: &RuleScope,
    ) -> Option<RuleVerdict> {
        None
    }

    fn fallback_hook_reply(&self) -> Option<String> {
        None
    }

    fn fallback_statusline(&self, payload: &str) -> String {
        minimal_statusline(&self.map_tap(payload).unwrap_or_default())
    }

    fn hookup(&self) -> Option<Box<dyn AgentHookup>> {
        None
    }

    fn agent_dirs(&self, _lookup: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
        Vec::new()
    }

    fn user_statusline_command(
        &self,
        _cwd: &Path,
        _lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Option<String> {
        None
    }
}

fn minimal_statusline(events: &[AgentEvent]) -> String {
    let sample = events.iter().find_map(|event| match event {
        AgentEvent::UsageSample(sample) => Some(sample),
        _ => None,
    });
    let model = sample
        .and_then(|sample| sample.model.clone())
        .unwrap_or_else(|| "?".into());
    let context = sample
        .and_then(|sample| sample.context_used_percent)
        .map_or_else(|| "?".into(), |percent| format!("{percent:.0}"));
    format!("{model} · {context}%\n")
}
