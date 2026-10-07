use std::path::{Path, PathBuf};

use orch_core::{
    AgentEvent, ConversationId, PermissionMode, SessionId, SessionStatus, SubagentId,
    TranscriptEntry,
};

use crate::{GuardAnswer, Preset};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub session: SessionId,
    pub orch_program: String,
    pub preset: Preset,
    pub prompt: Option<String>,
    pub worktree: Option<PathBuf>,
}

impl LaunchSpec {
    pub fn new(session: SessionId, orch_program: impl Into<String>, preset: Preset) -> Self {
        Self {
            session,
            orch_program: orch_program.into(),
            preset,
            prompt: None,
            worktree: None,
        }
    }

    pub fn with_prompt(self, prompt: impl Into<String>) -> Self {
        Self {
            prompt: Some(prompt.into()),
            ..self
        }
    }

    pub fn with_worktree(self, worktree: impl Into<PathBuf>) -> Self {
        Self {
            worktree: Some(worktree.into()),
            ..self
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeRequest {
    Create { name: String },
    Remove { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadError(pub String);

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

pub trait AgentAdapter {
    fn capabilities(&self) -> Capabilities;

    fn launch(&self, spec: &LaunchSpec) -> Argv;

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

    fn draft(&self, _conversation: &ConversationId) -> Option<Argv> {
        None
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

    fn is_guard_payload(&self, _payload: &str) -> bool {
        false
    }

    fn guard_answer(&self, _answer: &GuardAnswer) -> Option<String> {
        None
    }

    fn worktree_request(&self, _payload: &str) -> Option<WorktreeRequest> {
        None
    }

    fn agent_dirs(&self, _lookup: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
        Vec::new()
    }
}
