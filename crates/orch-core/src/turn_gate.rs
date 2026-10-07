use crate::{AgentEvent, AgentState, Observation};

#[derive(Debug, Clone, Default)]
pub(crate) struct TurnGate {
    turn_reported: bool,
    turn_closed: bool,
    permission_pending: bool,
}

impl TurnGate {
    pub(crate) fn ignores(&self, observation: &Observation) -> bool {
        let Observation::Agent(event) = observation else {
            return false;
        };
        match event {
            AgentEvent::AwaitingPrompt => self.turn_reported,
            AgentEvent::PermissionRequested => self.turn_closed,
            AgentEvent::PermissionCleared => !self.permission_pending,
            AgentEvent::ToolStarted { .. } | AgentEvent::SubagentStarted { .. } => {
                self.permission_pending
            }
            _ => false,
        }
    }

    pub(crate) fn note(&mut self, observation: &Observation, after: AgentState) {
        match observation {
            Observation::Spawned => *self = Self::default(),
            Observation::Agent(event) if event.is_turn_activity() => {
                self.turn_reported = true;
                match event {
                    AgentEvent::TurnEnded | AgentEvent::Failed { .. } => self.turn_closed = true,
                    AgentEvent::PromptSubmitted => self.turn_closed = false,
                    _ => {}
                }
            }
            _ => {}
        }
        self.permission_pending = match observation {
            Observation::Agent(AgentEvent::PermissionRequested) => after == AgentState::NeedsInput,
            Observation::Agent(AgentEvent::QuestionAsked) | Observation::GuardPrompted => false,
            _ => self.permission_pending && after == AgentState::NeedsInput,
        };
    }
}
