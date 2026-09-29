mod common;

use std::time::{Duration, Instant};

use common::*;
use orch_core::{
    AgentEvent, AgentState, Attention, ChecksState, DEFAULT_STALLED_AFTER, Effect, FailureKind,
    Flags, Observation, PhaseEvent, PrState, PrStatus, ReviewDecision, SessionStatus,
};

fn attention(effects: &[Effect]) -> Vec<Attention> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Attention(attention) => Some(*attention),
            _ => None,
        })
        .collect()
}

#[test]
fn finishing_a_turn_needing_input_or_failing_calls_for_attention() {
    let mut status = working_session();
    assert_eq!(
        attention(&status.feed_event(AgentEvent::PermissionRequested)),
        [Attention::NeedsInput]
    );
    assert_eq!(
        attention(&status.feed_event(AgentEvent::TurnEnded)),
        [Attention::TurnEnded]
    );

    let mut status = working_session();
    let effects = status.feed_event(AgentEvent::Failed {
        kind: FailureKind::Server,
    });
    assert_eq!(attention(&effects), [Attention::Errored]);

    let mut status = working_session();
    assert_eq!(
        attention(&status.feed(Observation::Exited { code: Some(2) })),
        [Attention::Errored]
    );
}

#[test]
fn staying_in_a_state_or_working_calls_for_no_attention() {
    let mut status = working_session();
    assert!(attention(&status.feed_event(AgentEvent::PromptSubmitted)).is_empty());
    let mut status = idle_session();
    assert!(attention(&status.feed_event(AgentEvent::TurnEnded)).is_empty());
    let mut status = working_session();
    assert!(attention(&status.feed(Observation::Exited { code: Some(0) })).is_empty());
}

#[test]
fn a_failed_setup_calls_for_attention() {
    let mut status = SessionStatus::new();
    let effects = status.transition(PhaseEvent::SetupFailed).unwrap();
    assert_eq!(attention(&effects), [Attention::SetupFailed]);
    assert!(status.flags().unseen);
}

#[test]
fn attention_while_unwatched_marks_the_session_unseen_until_seen() {
    let mut status = working_session();
    status.feed_event(AgentEvent::TurnEnded);
    assert!(status.flags().unseen);

    status.set_watched(true);
    assert!(!status.flags().unseen);
}

#[test]
fn attention_while_watched_leaves_the_session_seen() {
    let mut status = working_session();
    status.set_watched(true);
    status.feed_event(AgentEvent::PermissionRequested);
    assert!(!status.flags().unseen);

    status.set_watched(false);
    status.feed_event(AgentEvent::ToolStarted {
        tool: "Bash".into(),
        subagent: None,
    });
    assert!(!status.flags().unseen);
}

#[test]
fn a_working_agent_without_activity_becomes_stalled() {
    let start = Instant::now();
    let mut status = idle_session();
    status.observe(Observation::Agent(AgentEvent::PromptSubmitted), start);

    status.tick(
        start + DEFAULT_STALLED_AFTER - Duration::from_secs(1),
        DEFAULT_STALLED_AFTER,
    );
    assert!(!status.flags().stalled);
    status.tick(start + DEFAULT_STALLED_AFTER, DEFAULT_STALLED_AFTER);
    assert!(status.flags().stalled);
}

#[test]
fn activity_clears_stalled_and_usage_samples_are_not_activity() {
    let start = Instant::now();
    let mut status = idle_session();
    status.observe(Observation::Agent(AgentEvent::PromptSubmitted), start);
    status.observe(
        Observation::Agent(AgentEvent::UsageSample(Default::default())),
        start + DEFAULT_STALLED_AFTER / 2,
    );
    status.tick(start + DEFAULT_STALLED_AFTER, DEFAULT_STALLED_AFTER);
    assert!(status.flags().stalled);

    status.observe(Observation::UserInput, start + DEFAULT_STALLED_AFTER);
    assert!(!status.flags().stalled);
}

#[test]
fn only_working_agents_stall() {
    let start = Instant::now();
    let mut status = idle_session();
    status.observe(Observation::Agent(AgentEvent::PermissionRequested), start);
    status.tick(start + 2 * DEFAULT_STALLED_AFTER, DEFAULT_STALLED_AFTER);
    assert!(!status.flags().stalled);
    assert_eq!(status.agent_state(), Some(AgentState::NeedsInput));
}

#[test]
fn needs_rebase_is_rechecked_after_each_turn_and_cleared_once_the_base_tip_is_contained() {
    let mut status = working_session();
    assert!(
        !status
            .feed_event(AgentEvent::TurnEnded)
            .contains(&Effect::RecheckRebase)
    );

    status.flag_needs_rebase();
    assert!(status.flags().needs_rebase);
    status.feed_event(AgentEvent::PromptSubmitted);
    assert!(
        status
            .feed_event(AgentEvent::TurnEnded)
            .contains(&Effect::RecheckRebase)
    );

    status.rebase_checked(false);
    assert!(status.flags().needs_rebase);
    status.rebase_checked(true);
    assert!(!status.flags().needs_rebase);
}

fn pr(checks: ChecksState, review: ReviewDecision, new_comments: u32, state: PrState) -> PrStatus {
    PrStatus {
        number: 42,
        checks,
        review,
        new_comments,
        state,
    }
}

fn pr_session() -> SessionStatus {
    let mut status = idle_session();
    status.transition(OPEN_PR).unwrap();
    status
}

#[test]
fn pr_updates_are_recorded_and_failing_checks_or_requested_changes_call_for_attention() {
    let mut status = pr_session();
    let pending = pr(
        ChecksState::Pending,
        ReviewDecision::ReviewRequired,
        0,
        PrState::Open,
    );
    assert!(attention(&status.update_pr(pending.clone())).is_empty());
    assert_eq!(status.flags().pr, Some(pending));

    let failing = pr(
        ChecksState::Failing,
        ReviewDecision::ChangesRequested,
        2,
        PrState::Open,
    );
    assert_eq!(
        attention(&status.update_pr(failing.clone())),
        [Attention::ChecksFailing, Attention::ChangesRequested]
    );
    assert!(attention(&status.update_pr(failing)).is_empty());
}

#[test]
fn a_pr_closed_without_merging_is_flagged_for_a_decision() {
    let mut status = pr_session();
    let closed = pr(
        ChecksState::Passing,
        ReviewDecision::None,
        0,
        PrState::Closed,
    );
    assert_eq!(attention(&status.update_pr(closed)), [Attention::PrClosed]);
    assert_eq!(
        status.flags().pr.as_ref().map(|pr| pr.state),
        Some(PrState::Closed)
    );
}

#[test]
fn landing_a_pr_session_reports_the_merge() {
    let mut status = pr_session();
    let effects = status.transition(PhaseEvent::PrMerged).unwrap();
    assert_eq!(attention(&effects), [Attention::PrMerged]);

    let mut status = idle_session();
    assert!(attention(&status.transition(PhaseEvent::Landed).unwrap()).is_empty());
}

#[test]
fn muted_and_recovered_sessions_still_derive_state_and_call_for_attention() {
    let mut status = working_session();
    status.set_muted(true);
    status.set_recovered(true);

    let effects = status.feed_event(AgentEvent::TurnEnded);

    assert_eq!(status.agent_state(), Some(AgentState::Idle));
    assert_eq!(attention(&effects), [Attention::TurnEnded]);
    let flags = status.flags();
    assert!(flags.muted && flags.recovered && flags.unseen);
}

#[test]
fn unmuting_and_fixed_worktree_and_base_are_reflected_in_the_flags() {
    let mut status = idle_session();
    status.set_muted(true);
    status.set_worktree_missing(true);
    status.set_base_missing(true);
    assert!(status.flags().worktree_missing && status.flags().base_missing);

    status.set_muted(false);
    status.set_worktree_missing(false);
    status.set_base_missing(false);
    assert_eq!(status.flags(), &Flags::default());
}

#[test]
fn a_status_restored_from_records_keeps_phase_and_flags_until_observed_again() {
    let flags = Flags {
        unseen: true,
        muted: true,
        pr: Some(PrStatus::opened(7)),
        ..Flags::default()
    };
    let mut status = SessionStatus::new();
    status.restore(orch_core::Phase::PrOpen, flags.clone());
    assert_eq!(status.phase(), orch_core::Phase::PrOpen);
    assert_eq!(status.flags(), &flags);
    assert_eq!(status.agent_state(), None);

    status.feed(Observation::Spawned);
    assert_eq!(status.agent_state(), Some(AgentState::Starting));
    status.transition(PhaseEvent::PrMerged).unwrap();
    assert_eq!(status.phase(), orch_core::Phase::Landed);
}
