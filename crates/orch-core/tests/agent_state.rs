mod common;

use common::*;
use orch_core::{
    AgentEvent, AgentState, ConversationId, FailureKind, Observation, PermissionMode, Phase,
    SessionStatus, SubagentId, UsageSample,
};

#[test]
fn new_session_is_setting_up_without_agent_state() {
    let status = SessionStatus::new();
    assert_eq!(status.phase(), Phase::SettingUp);
    assert_eq!(status.agent_state(), None);
}

#[test]
fn spawned_agent_is_starting_then_idle_on_session_started() {
    let mut status = active_session();
    assert_eq!(status.phase(), Phase::Active);

    status.feed(Observation::Spawned);
    assert_eq!(status.agent_state(), Some(AgentState::Starting));

    status.feed_event(AgentEvent::SessionStarted);
    assert_eq!(status.agent_state(), Some(AgentState::Idle));
}

fn tool(name: &str) -> (String, Option<SubagentId>) {
    (name.to_string(), None)
}

#[test]
fn activity_events_make_the_agent_working() {
    let (bash, main) = tool("Bash");
    let events = [
        AgentEvent::PromptSubmitted,
        AgentEvent::ToolStarted {
            tool: bash.clone(),
            subagent: main.clone(),
        },
        AgentEvent::ToolFinished {
            tool: bash,
            subagent: main,
        },
        AgentEvent::SubagentStarted {
            id: SubagentId("a1".into()),
            agent_type: "Explore".into(),
            description: "find things".into(),
        },
        AgentEvent::PermissionDenied,
    ];
    for event in events {
        let mut status = idle_session();
        status.feed_event(event.clone());
        assert_eq!(status.agent_state(), Some(AgentState::Working), "{event:?}");
    }
}

#[test]
fn permission_requests_and_questions_need_input() {
    for event in [AgentEvent::PermissionRequested, AgentEvent::QuestionAsked] {
        let mut status = idle_session();
        status.feed_event(AgentEvent::PromptSubmitted);
        status.feed_event(event.clone());
        assert_eq!(
            status.agent_state(),
            Some(AgentState::NeedsInput),
            "{event:?}"
        );
    }
}

#[test]
fn needs_input_clears_on_the_next_event() {
    let mut status = idle_session();
    status.feed_event(AgentEvent::PermissionRequested);
    status.feed_event(AgentEvent::ToolStarted {
        tool: "Edit".into(),
        subagent: None,
    });
    assert_eq!(status.agent_state(), Some(AgentState::Working));
}

#[test]
fn turn_ended_makes_the_agent_idle() {
    let mut status = idle_session();
    status.feed_event(AgentEvent::PromptSubmitted);
    status.feed_event(AgentEvent::TurnEnded);
    assert_eq!(status.agent_state(), Some(AgentState::Idle));
}

#[test]
fn failure_or_non_zero_exit_is_errored() {
    let mut status = idle_session();
    status.feed_event(AgentEvent::PromptSubmitted);
    status.feed_event(AgentEvent::Failed {
        kind: FailureKind::RateLimited,
    });
    assert_eq!(status.agent_state(), Some(AgentState::Errored));

    for code in [Some(1), None] {
        let mut status = idle_session();
        status.feed(Observation::Exited { code });
        assert_eq!(status.agent_state(), Some(AgentState::Errored), "{code:?}");
    }
}

#[test]
fn zero_exit_is_exited() {
    let mut status = idle_session();
    status.feed(Observation::Exited { code: Some(0) });
    assert_eq!(status.agent_state(), Some(AgentState::Exited));
}

#[test]
fn bookkeeping_events_keep_the_agent_state() {
    let events = [
        AgentEvent::ModeChanged {
            mode: PermissionMode::Plan,
        },
        AgentEvent::ConversationChanged {
            id: ConversationId("c2".into()),
        },
        AgentEvent::UsageSample(UsageSample::default()),
        AgentEvent::SubagentFinished {
            id: SubagentId("a1".into()),
        },
        AgentEvent::GuardCheck {
            tool: "Bash".into(),
            input_json: "{}".into(),
        },
    ];
    for event in events {
        let mut status = idle_session();
        status.feed_event(AgentEvent::PermissionRequested);
        status.feed_event(event.clone());
        assert_eq!(
            status.agent_state(),
            Some(AgentState::NeedsInput),
            "{event:?}"
        );
    }
}

#[test]
fn a_guard_prompt_needs_input() {
    let mut status = idle_session();
    status.feed_event(AgentEvent::PromptSubmitted);
    status.feed(Observation::GuardPrompted);
    assert_eq!(status.agent_state(), Some(AgentState::NeedsInput));
}

#[test]
fn user_input_sets_working_provisionally_while_agent_waits() {
    for setup in [AgentEvent::TurnEnded, AgentEvent::PermissionRequested] {
        let mut status = idle_session();
        status.feed_event(setup);
        status.feed(Observation::UserInput);
        assert_eq!(status.agent_state(), Some(AgentState::Working));
    }
}

#[test]
fn user_input_sets_working_provisionally_on_an_errored_but_alive_agent() {
    let mut status = working_session();
    status.feed_event(AgentEvent::Failed {
        kind: FailureKind::Server,
    });
    status.feed(Observation::UserInput);
    assert_eq!(status.agent_state(), Some(AgentState::Working));
}

#[test]
fn user_input_does_not_change_a_starting_agent() {
    let mut status = active_session();
    status.feed(Observation::Spawned);
    status.feed(Observation::UserInput);
    assert_eq!(status.agent_state(), Some(AgentState::Starting));
}

#[test]
fn nothing_but_a_respawn_changes_an_agent_after_its_process_exited() {
    let mut status = idle_session();
    status.feed(Observation::Exited { code: Some(1) });
    status.feed(Observation::UserInput);
    status.feed_event(AgentEvent::PromptSubmitted);
    assert_eq!(status.agent_state(), Some(AgentState::Errored));

    status.feed(Observation::Spawned);
    assert_eq!(status.agent_state(), Some(AgentState::Starting));
}

#[test]
fn observations_are_ignored_outside_live_phases() {
    let mut status = SessionStatus::new();
    status.feed(Observation::Spawned);
    assert_eq!(status.agent_state(), None);
}
