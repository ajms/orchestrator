use orch_core::{AgentEvent, ConversationId, PermissionMode, SessionId, SessionStatus};

use crate::{GuardAnswer, Preset};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub hooks: bool,
    pub resume: bool,
    pub usage: bool,
    pub modes: bool,
    pub guards: bool,
    pub subagents: bool,
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

    pub fn with_prompt(self, prompt: impl Into<String>) -> Self {
        Self {
            prompt: Some(prompt.into()),
            ..self
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadError(pub String);

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

    fn is_guard_payload(&self, _payload: &str) -> bool {
        false
    }

    fn guard_answer(&self, _answer: &GuardAnswer) -> Option<String> {
        None
    }
}
