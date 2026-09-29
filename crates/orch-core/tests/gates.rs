mod common;

use common::*;
use orch_core::{AgentState, DiscardPlan, GateRefusal, Phase, PhaseEvent, SessionStatus};

#[test]
fn landing_is_allowed_when_the_agent_is_idle_exited_or_errored() {
    for status in [idle_session(), exited_session(0), exited_session(1)] {
        assert_eq!(status.check_landing(), Ok(()), "{:?}", status.agent_state());
    }
}

#[test]
fn landing_is_blocked_while_the_agent_is_busy() {
    assert_eq!(
        working_session().check_landing(),
        Err(GateRefusal::AgentBusy {
            state: Some(AgentState::Working)
        })
    );
    assert_eq!(
        needs_input_session().check_landing(),
        Err(GateRefusal::AgentBusy {
            state: Some(AgentState::NeedsInput)
        })
    );
    assert_eq!(
        starting_session().check_landing(),
        Err(GateRefusal::AgentBusy {
            state: Some(AgentState::Starting)
        })
    );
}

#[test]
fn landing_is_refused_in_active_before_the_agent_is_spawned() {
    assert_eq!(
        active_session().check_landing(),
        Err(GateRefusal::AgentBusy { state: None })
    );
}

#[test]
fn a_suspended_session_can_be_landed() {
    let status = with_phase(working_session(), &[PhaseEvent::Suspended]);
    assert_eq!(status.check_landing(), Ok(()));
}

#[test]
fn landing_needs_a_session_that_has_not_landed_or_opened_a_pr() {
    assert_eq!(
        SessionStatus::new().check_landing(),
        Err(GateRefusal::WrongPhase {
            phase: Phase::SettingUp
        })
    );
    assert_eq!(
        with_phase(idle_session(), &[OPEN_PR]).check_landing(),
        Err(GateRefusal::WrongPhase {
            phase: Phase::PrOpen
        })
    );
    assert_eq!(
        with_phase(idle_session(), &[OPEN_PR, PhaseEvent::Suspended]).check_landing(),
        Err(GateRefusal::WrongPhase {
            phase: Phase::Suspended
        })
    );
    assert_eq!(
        with_phase(idle_session(), &[PhaseEvent::Landed]).check_landing(),
        Err(GateRefusal::WrongPhase {
            phase: Phase::Landed
        })
    );
}

#[test]
fn changing_the_preset_is_allowed_exactly_when_the_agent_would_allow_landing() {
    assert_eq!(idle_session().check_preset_change(), Ok(()));
    assert_eq!(exited_session(1).check_preset_change(), Ok(()));
    assert_eq!(
        with_phase(working_session(), &[PhaseEvent::Suspended]).check_preset_change(),
        Ok(())
    );
    assert_eq!(
        with_phase(idle_session(), &[OPEN_PR]).check_preset_change(),
        Ok(())
    );
    assert_eq!(
        working_session().check_preset_change(),
        Err(GateRefusal::AgentBusy {
            state: Some(AgentState::Working)
        })
    );
    assert_eq!(
        SessionStatus::new().check_preset_change(),
        Err(GateRefusal::WrongPhase {
            phase: Phase::SettingUp
        })
    );
}

#[test]
fn discarding_is_always_allowed_and_stops_a_running_agent_first() {
    let stops_agent = DiscardPlan {
        stop_agent: true,
        stop_setup: false,
    };
    let nothing_to_stop = DiscardPlan {
        stop_agent: false,
        stop_setup: false,
    };
    assert_eq!(working_session().check_discard(), Ok(stops_agent));
    assert_eq!(needs_input_session().check_discard(), Ok(stops_agent));
    assert_eq!(idle_session().check_discard(), Ok(stops_agent));
    assert_eq!(exited_session(0).check_discard(), Ok(nothing_to_stop));
    assert_eq!(
        with_phase(working_session(), &[PhaseEvent::Suspended]).check_discard(),
        Ok(nothing_to_stop)
    );
    assert_eq!(
        session_through(&[PhaseEvent::SetupFailed]).check_discard(),
        Ok(nothing_to_stop)
    );
}

#[test]
fn discarding_while_setting_up_stops_the_setup_script() {
    assert_eq!(
        SessionStatus::new().check_discard(),
        Ok(DiscardPlan {
            stop_agent: false,
            stop_setup: true
        })
    );
}

#[test]
fn a_finished_session_cannot_be_discarded() {
    for (events, phase) in [
        (&[PhaseEvent::Landed][..], Phase::Landed),
        (&[PhaseEvent::Discarded], Phase::Discarded),
    ] {
        assert_eq!(
            with_phase(idle_session(), events).check_discard(),
            Err(GateRefusal::WrongPhase { phase })
        );
    }
}
