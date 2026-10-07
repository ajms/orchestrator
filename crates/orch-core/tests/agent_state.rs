mod common;

use common::*;
use orch_core::{
    AgentEvent, AgentState, ConversationId, FailureKind, GuardedAction, Observation,
    PermissionMode, Phase, SessionStatus, SubagentId, UsageSample,
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
    status.feed_event(AgentEvent::ToolFinished {
        tool: "Edit".into(),
        subagent: None,
    });
    assert_eq!(status.agent_state(), Some(AgentState::Working));
}

#[test]
fn a_tool_starting_elsewhere_keeps_a_pending_permission_prompt() {
    let mut status = working_session();
    status.feed_event(AgentEvent::PermissionRequested);
    status.feed_event(AgentEvent::ToolStarted {
        tool: "run_command".into(),
        subagent: None,
    });
    assert_eq!(status.agent_state(), Some(AgentState::NeedsInput));

    status.feed_event(AgentEvent::PromptSubmitted);
    assert_eq!(status.agent_state(), Some(AgentState::Working));
}

#[test]
fn a_late_permission_request_does_not_reopen_a_finished_turn() {
    let mut idle = working_session();
    idle.feed_event(AgentEvent::TurnEnded);
    let mut errored = working_session();
    errored.feed_event(AgentEvent::Failed {
        kind: FailureKind::Server,
    });
    for (mut status, settled) in [(idle, AgentState::Idle), (errored, AgentState::Errored)] {
        status.feed_event(AgentEvent::PermissionRequested);
        assert_eq!(status.agent_state(), Some(settled));

        status.feed_event(AgentEvent::PromptSubmitted);
        status.feed_event(AgentEvent::PermissionRequested);
        assert_eq!(status.agent_state(), Some(AgentState::NeedsInput));
    }
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
            action: GuardedAction::Shell {
                command: "ls".into(),
            },
            cwd: None,
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

#[test]
fn an_adopted_agent_resumes_from_its_persisted_state() {
    let mut status = active_session();
    status.restore_agent(AgentState::Idle, true);
    assert_eq!(status.agent_state(), Some(AgentState::Idle));
    status.feed_event(AgentEvent::PromptSubmitted);
    assert_eq!(status.agent_state(), Some(AgentState::Working));

    let mut status = active_session();
    status.restore_agent(AgentState::Idle, false);
    status.feed_event(AgentEvent::PromptSubmitted);
    assert_eq!(status.agent_state(), Some(AgentState::Idle));
}

#[test]
fn awaiting_a_prompt_before_the_first_turn_settles_the_agent_as_idle() {
    let mut status = starting_session();
    status.feed_event(AgentEvent::AwaitingPrompt);
    assert_eq!(status.agent_state(), Some(AgentState::Idle));

    let mut status = starting_session();
    status.feed_event(AgentEvent::PermissionRequested);
    status.feed(Observation::UserInput);
    status.feed_event(AgentEvent::AwaitingPrompt);
    assert_eq!(status.agent_state(), Some(AgentState::Idle));
}

#[test]
fn awaiting_a_prompt_after_a_turn_began_keeps_the_agent_state() {
    let mut status = starting_session();
    status.feed_event(AgentEvent::PromptSubmitted);
    status.feed_event(AgentEvent::AwaitingPrompt);
    assert_eq!(status.agent_state(), Some(AgentState::Working));

    status.feed_event(AgentEvent::Failed {
        kind: FailureKind::Other("error".into()),
    });
    status.feed_event(AgentEvent::AwaitingPrompt);
    assert_eq!(status.agent_state(), Some(AgentState::Errored));
}

#[test]
fn a_respawned_agent_can_await_a_prompt_again() {
    let mut status = working_session();
    status.feed(Observation::Exited { code: Some(0) });
    status.feed(Observation::Spawned);
    status.feed_event(AgentEvent::AwaitingPrompt);
    assert_eq!(status.agent_state(), Some(AgentState::Idle));
}

#[test]
fn a_cleared_permission_prompt_leaves_needs_input() {
    let mut status = working_session();
    status.feed_event(AgentEvent::PermissionRequested);
    status.feed_event(AgentEvent::PermissionCleared);
    assert_eq!(status.agent_state(), Some(AgentState::Working));
}

#[test]
fn a_cleared_permission_prompt_leaves_questions_guards_and_other_states_alone() {
    let mut asked = working_session();
    asked.feed_event(AgentEvent::PermissionRequested);
    asked.feed_event(AgentEvent::QuestionAsked);
    let mut guarded = working_session();
    guarded.feed(Observation::GuardPrompted);
    for mut status in [asked, guarded] {
        status.feed_event(AgentEvent::PermissionCleared);
        assert_eq!(status.agent_state(), Some(AgentState::NeedsInput));
    }

    let mut status = idle_session();
    status.feed_event(AgentEvent::PermissionCleared);
    assert_eq!(status.agent_state(), Some(AgentState::Idle));
}
