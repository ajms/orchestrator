use crate::{AgentEvent, Observation};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentState {
    Starting,
    Working,
    NeedsInput,
    Idle,
    Errored,
    Exited,
}

impl AgentState {
    pub(crate) fn after(self, observation: &Observation) -> AgentState {
        match observation {
            Observation::Spawned => AgentState::Starting,
            Observation::Exited { code: Some(0) } => AgentState::Exited,
            Observation::Exited { .. } => AgentState::Errored,
            Observation::GuardPrompted => AgentState::NeedsInput,
            Observation::UserInput => match self {
                AgentState::Idle | AgentState::NeedsInput | AgentState::Errored => {
                    AgentState::Working
                }
                other => other,
            },
            Observation::Agent(event) => self.after_event(event),
        }
    }

    fn after_event(self, event: &AgentEvent) -> AgentState {
        match event {
            AgentEvent::SessionStarted | AgentEvent::TurnEnded => AgentState::Idle,
            AgentEvent::PromptSubmitted
            | AgentEvent::ToolStarted { .. }
            | AgentEvent::ToolFinished { .. }
            | AgentEvent::SubagentStarted { .. }
            | AgentEvent::PermissionDenied => AgentState::Working,
            AgentEvent::PermissionRequested | AgentEvent::QuestionAsked => AgentState::NeedsInput,
            AgentEvent::Failed { .. } => AgentState::Errored,
            AgentEvent::ModeChanged { .. }
            | AgentEvent::ConversationChanged { .. }
            | AgentEvent::UsageSample(_)
            | AgentEvent::SubagentFinished { .. }
            | AgentEvent::GuardCheck { .. } => self,
        }
    }
}
