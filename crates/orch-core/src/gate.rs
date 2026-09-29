use crate::{AgentState, Phase};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateRefusal {
    WrongPhase { phase: Phase },
    AgentBusy { state: Option<AgentState> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscardPlan {
    pub stop_agent: bool,
    pub stop_setup: bool,
}

pub(crate) fn agent_settled(state: Option<AgentState>) -> Result<(), GateRefusal> {
    match state {
        Some(AgentState::Idle | AgentState::Exited | AgentState::Errored) => Ok(()),
        state => Err(GateRefusal::AgentBusy { state }),
    }
}
