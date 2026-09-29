mod common;

use common::*;
use orch_core::{AgentState, GateRefusal, Observation, PhaseEvent, SessionStatus};

fn unobserved_session() -> SessionStatus {
    with_phase(SessionStatus::unobserved(), &[PhaseEvent::SetupSucceeded])
}

#[test]
fn an_unobserved_spawned_agent_is_in_an_unknown_state() {
    let mut status = unobserved_session();
    status.feed(Observation::Spawned);
    assert_eq!(status.agent_state(), Some(AgentState::Unknown));

    status.feed(Observation::UserInput);
    assert_eq!(status.agent_state(), Some(AgentState::Unknown));
}

#[test]
fn process_exit_is_still_known_for_an_unobserved_agent() {
    for (code, expected) in [(0, AgentState::Exited), (1, AgentState::Errored)] {
        let mut status = unobserved_session();
        status.feed(Observation::Spawned);
        status.feed(Observation::Exited { code: Some(code) });
        assert_eq!(status.agent_state(), Some(expected));
    }
}

#[test]
fn landing_waits_for_an_agent_in_an_unknown_state_to_exit() {
    let mut status = unobserved_session();
    status.feed(Observation::Spawned);
    assert_eq!(
        status.check_landing(),
        Err(GateRefusal::AgentBusy {
            state: Some(AgentState::Unknown)
        })
    );

    status.feed(Observation::Exited { code: Some(0) });
    assert_eq!(status.check_landing(), Ok(()));
}
