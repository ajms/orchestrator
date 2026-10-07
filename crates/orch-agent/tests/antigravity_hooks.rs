use std::time::Instant;

use orch_agent::hook_event::tag_hook_event;
use orch_agent::{AgentAdapter, Antigravity};
use orch_core::{AgentEvent, AgentState, FailureKind, Observation, PhaseEvent};

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/antigravity/hooks/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path}: {err}"))
}

fn hook(event: &str, name: &str) -> String {
    tag_hook_event(&fixture(name), event)
}

fn events(event: &str, name: &str) -> Vec<AgentEvent> {
    Antigravity::default()
        .map_hook(&hook(event, name))
        .unwrap_or_else(|err| panic!("{name}: {err:?}"))
}

fn failed(reason: &str) -> Vec<AgentEvent> {
    vec![AgentEvent::Failed {
        kind: FailureKind::Other(reason.into()),
    }]
}

#[test]
fn a_model_invocation_means_the_agent_is_working() {
    assert_eq!(
        events("PreInvocation", "pre_invocation"),
        [AgentEvent::PromptSubmitted]
    );
}

#[test]
fn a_finished_invocation_leaves_errored_and_needs_input_alone() {
    let settle = |event: AgentEvent| {
        let mut status = Antigravity::default().capabilities().session_status();
        status.transition(PhaseEvent::SetupSucceeded).unwrap();
        status.observe(Observation::Spawned, Instant::now());
        status.observe(Observation::Agent(event), Instant::now());
        status
    };
    let errored = settle(AgentEvent::Failed {
        kind: FailureKind::Server,
    });
    let asked = settle(AgentEvent::QuestionAsked);
    for (mut status, held) in [
        (errored, AgentState::Errored),
        (asked, AgentState::NeedsInput),
    ] {
        for event in events("PostInvocation", "post_invocation") {
            status.observe(Observation::Agent(event), Instant::now());
        }
        assert_eq!(status.agent_state(), Some(held));
    }
}

#[test]
fn tool_hooks_start_and_finish_tools() {
    assert_eq!(
        events("PreToolUse", "pre_tool_use_run_command").first(),
        Some(&AgentEvent::ToolStarted {
            tool: "run_command".into(),
            subagent: None,
        })
    );
    assert_eq!(
        events("PostToolUse", "post_tool_use"),
        [AgentEvent::ToolFinished {
            tool: "run_command".into(),
            subagent: None,
        }]
    );
}

#[test]
fn the_ask_question_tool_asks_the_user() {
    assert_eq!(
        events("PreToolUse", "pre_tool_use_ask_question"),
        [AgentEvent::QuestionAsked]
    );
}

#[test]
fn only_a_fully_idle_stop_ends_the_turn() {
    assert_eq!(events("Stop", "stop_fully_idle"), [AgentEvent::TurnEnded]);
    assert_eq!(events("Stop", "stop_waiting_on_subagent"), []);
}

#[test]
fn a_stop_that_errored_or_ran_out_of_steps_fails() {
    assert_eq!(
        events("Stop", "stop_error"),
        failed("model request failed: RESOURCE_EXHAUSTED")
    );
    assert_eq!(
        events("Stop", "stop_max_steps_exceeded"),
        failed("max_steps_exceeded")
    );
}

#[test]
fn only_pre_tool_use_blocks_on_a_guard() {
    let agy = Antigravity::default();
    assert!(agy.is_guard_payload(&hook("PreToolUse", "pre_tool_use_run_command")));
    assert!(!agy.is_guard_payload(&hook("PostToolUse", "post_tool_use")));
    assert!(!agy.is_guard_payload(&hook("Stop", "stop_fully_idle")));
}

#[test]
fn a_pre_tool_use_from_a_hook_without_an_event_name_still_blocks() {
    let agy = Antigravity::default();
    assert!(agy.is_guard_payload(&fixture("pre_tool_use_run_command")));
    assert!(!agy.is_guard_payload(&fixture("post_tool_use")));
}

#[test]
fn unknown_hooks_are_ignored_and_garbage_is_an_error() {
    let agy = Antigravity::default();
    assert_eq!(
        agy.map_hook(&hook("SessionStart", "pre_invocation")),
        Ok(vec![])
    );
    assert_eq!(agy.map_hook(&fixture("stop_fully_idle")), Ok(vec![]));
    assert!(agy.map_hook("not json").is_err());
}
