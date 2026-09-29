mod common;

use common::*;
use orch_core::{AgentEvent, InvalidTransition, Observation, Phase, PhaseEvent, SessionStatus};

fn phase_after(events: &[PhaseEvent]) -> Phase {
    session_through(events).phase()
}

use PhaseEvent::*;

#[test]
fn setup_leads_to_active_or_setup_failed() {
    assert_eq!(phase_after(&[SetupSucceeded]), Phase::Active);
    assert_eq!(phase_after(&[SetupFailed]), Phase::SetupFailed);
}

#[test]
fn failed_setup_can_be_retried_or_skipped() {
    assert_eq!(phase_after(&[SetupFailed, SetupRetried]), Phase::SettingUp);
    assert_eq!(phase_after(&[SetupFailed, SetupSkipped]), Phase::Active);
}

#[test]
fn active_lands_directly_or_through_an_open_pr() {
    assert_eq!(phase_after(&[SetupSucceeded, Landed]), Phase::Landed);
    assert_eq!(phase_after(&[SetupSucceeded, OPEN_PR]), Phase::PrOpen);
    assert_eq!(
        phase_after(&[SetupSucceeded, OPEN_PR, PrMerged]),
        Phase::Landed
    );
}

#[test]
fn active_and_pr_open_suspend_and_resume_to_where_they_were() {
    assert_eq!(phase_after(&[SetupSucceeded, Suspended]), Phase::Suspended);
    assert_eq!(
        phase_after(&[SetupSucceeded, Suspended, Resumed]),
        Phase::Active
    );
    assert_eq!(
        phase_after(&[SetupSucceeded, OPEN_PR, Suspended, Resumed]),
        Phase::PrOpen
    );
}

#[test]
fn a_pr_session_lands_only_by_its_pr_being_merged() {
    assert!(
        session_through(&[SetupSucceeded, OPEN_PR])
            .transition(Landed)
            .is_err()
    );
    assert!(
        session_through(&[SetupSucceeded])
            .transition(PrMerged)
            .is_err()
    );
    assert_eq!(
        phase_after(&[SetupSucceeded, OPEN_PR, Suspended, PrMerged]),
        Phase::Landed
    );
}

#[test]
fn discarding_is_possible_while_setting_up() {
    assert_eq!(phase_after(&[Discarded]), Phase::Discarded);
}

#[test]
fn a_suspended_session_can_be_landed_or_get_a_pr() {
    assert_eq!(
        phase_after(&[SetupSucceeded, Suspended, Landed]),
        Phase::Landed
    );
    assert_eq!(
        phase_after(&[SetupSucceeded, Suspended, OPEN_PR, Resumed]),
        Phase::PrOpen
    );
}

#[test]
fn discarding_is_possible_from_active_pr_open_suspended_and_setup_failed() {
    for path in [
        &[SetupSucceeded][..],
        &[SetupSucceeded, OPEN_PR],
        &[SetupSucceeded, Suspended],
        &[SetupFailed],
    ] {
        let mut events = path.to_vec();
        events.push(Discarded);
        assert_eq!(phase_after(&events), Phase::Discarded, "{path:?}");
    }
}

#[test]
fn landed_and_discarded_are_terminal() {
    for terminal in [&[SetupSucceeded, Landed][..], &[SetupFailed, Discarded]] {
        let mut status = session_through(terminal);
        for event in [
            SetupSucceeded,
            OPEN_PR,
            Landed,
            PrMerged,
            Discarded,
            Suspended,
            Resumed,
        ] {
            assert!(
                status.transition(event).is_err(),
                "{terminal:?} then {event:?}"
            );
        }
    }
}

#[test]
fn invalid_transitions_are_refused_and_leave_the_phase_unchanged() {
    let mut status = SessionStatus::new();
    assert_eq!(
        status.transition(Landed),
        Err(InvalidTransition {
            from: Phase::SettingUp,
            event: Landed
        })
    );
    assert_eq!(status.phase(), Phase::SettingUp);
    assert!(
        session_through(&[SetupSucceeded])
            .transition(Resumed)
            .is_err()
    );
    assert!(
        session_through(&[SetupSucceeded, OPEN_PR])
            .transition(OPEN_PR)
            .is_err()
    );
}

#[test]
fn leaving_a_live_phase_forgets_the_agent_state() {
    let mut status = session_through(&[SetupSucceeded]);
    status.feed(Observation::Spawned);
    status.feed_event(AgentEvent::SessionStarted);
    status.transition(Suspended).unwrap();
    assert_eq!(status.agent_state(), None);

    status.transition(Resumed).unwrap();
    assert_eq!(status.agent_state(), None);
    status.feed(Observation::Spawned);
    assert!(status.agent_state().is_some());
}

#[test]
fn a_closed_pr_can_be_abandoned_to_return_the_session_to_active() {
    let mut status = session_through(&[SetupSucceeded, OPEN_PR]);
    assert!(status.transition(PrAbandoned).is_err());
    assert_eq!(status.phase(), Phase::PrOpen);

    let mut closed = orch_core::PrStatus::opened(42);
    closed.state = orch_core::PrState::Closed;
    status.update_pr(closed);
    status.transition(PrAbandoned).unwrap();

    assert_eq!(status.phase(), Phase::Active);
    assert_eq!(status.flags().pr, None);
    assert_eq!(
        status.transition(Landed).map(|_| status.phase()),
        Ok(Phase::Landed)
    );
}
