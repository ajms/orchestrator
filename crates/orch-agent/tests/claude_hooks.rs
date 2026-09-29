use orch_agent::{AgentAdapter, ClaudeCode, GuardAnswer};
use orch_core::{AgentEvent, ConversationId, FailureKind, PermissionMode, SubagentId};

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/hooks/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path}: {err}"))
}

fn events(name: &str) -> Vec<AgentEvent> {
    ClaudeCode::default()
        .map_hook(&fixture(name))
        .unwrap_or_else(|err| panic!("{name}: {err:?}"))
}

fn mode(mode: PermissionMode) -> AgentEvent {
    AgentEvent::ModeChanged { mode }
}

fn main_tool(event: fn(String, Option<SubagentId>) -> AgentEvent, tool: &str) -> AgentEvent {
    event(tool.into(), None)
}

fn started(tool: String, subagent: Option<SubagentId>) -> AgentEvent {
    AgentEvent::ToolStarted { tool, subagent }
}

fn finished(tool: String, subagent: Option<SubagentId>) -> AgentEvent {
    AgentEvent::ToolFinished { tool, subagent }
}

#[test]
fn session_start_reports_the_conversation_and_an_idle_agent() {
    assert_eq!(
        events("session_start_startup"),
        vec![
            mode(PermissionMode::Default),
            AgentEvent::ConversationChanged {
                id: ConversationId("5f0c6a52-6a3e-4d3b-9d7e-0b1f8c1e2a90".into())
            },
            AgentEvent::SessionStarted,
        ]
    );
}

#[test]
fn clear_moves_the_session_to_a_new_conversation() {
    assert_eq!(
        events("session_start_clear"),
        vec![
            mode(PermissionMode::AcceptEdits),
            AgentEvent::ConversationChanged {
                id: ConversationId("0d3f8a21-7b6c-4e5d-9a8b-1c2d3e4f5a6b".into())
            },
            AgentEvent::SessionStarted,
        ]
    );
}

#[test]
fn compaction_keeps_the_agent_state_because_it_can_happen_mid_turn() {
    assert_eq!(
        events("session_start_compact"),
        vec![
            mode(PermissionMode::Auto),
            AgentEvent::ConversationChanged {
                id: ConversationId("5f0c6a52-6a3e-4d3b-9d7e-0b1f8c1e2a90".into())
            },
        ]
    );
}

#[test]
fn prompt_submission_is_reported() {
    assert_eq!(
        events("user_prompt_submit"),
        vec![mode(PermissionMode::Default), AgentEvent::PromptSubmitted]
    );
}

#[test]
fn a_main_agent_tool_starts_and_asks_for_a_guard_check() {
    assert_eq!(
        events("pre_tool_use_bash"),
        vec![
            mode(PermissionMode::Default),
            main_tool(started, "Bash"),
            AgentEvent::GuardCheck {
                tool: "Bash".into(),
                input_json:
                    r#"{"command":"cargo test","description":"Run tests","timeout":120000}"#.into(),
                cwd: Some("/home/dev/shop/.orchestrator/worktrees/fix-login".into()),
            },
        ]
    );
}

#[test]
fn a_subagents_tool_is_attributed_to_it() {
    let events = events("pre_tool_use_in_subagent");
    assert_eq!(
        events[1],
        started("Grep".into(), Some(SubagentId("a7f3c9e1b2d4".into())))
    );
    assert!(matches!(events[2], AgentEvent::GuardCheck { .. }));
}

#[test]
fn finished_and_failed_tools_both_finish() {
    assert_eq!(
        events("post_tool_use_edit"),
        vec![
            mode(PermissionMode::AcceptEdits),
            main_tool(finished, "Edit")
        ]
    );
    assert_eq!(
        events("post_tool_use_failure_bash"),
        vec![mode(PermissionMode::Default), main_tool(finished, "Bash")]
    );
}

#[test]
fn a_permission_request_needs_input() {
    assert_eq!(
        events("permission_request_bash"),
        vec![
            mode(PermissionMode::Default),
            AgentEvent::PermissionRequested
        ]
    );
}

#[test]
fn a_question_to_the_user_is_asked_as_soon_as_the_tool_starts() {
    for fixture in [
        "pre_tool_use_ask_user_question",
        "permission_request_ask_user_question",
        "notification_elicitation_dialog",
        "notification_agent_needs_input",
    ] {
        assert_eq!(
            events(fixture),
            vec![mode(PermissionMode::Default), AgentEvent::QuestionAsked],
            "{fixture}"
        );
    }
}

#[test]
fn an_auto_mode_denial_is_reported() {
    assert_eq!(
        events("permission_denied_auto"),
        vec![mode(PermissionMode::Auto), AgentEvent::PermissionDenied]
    );
}

#[test]
fn delayed_permission_and_idle_notifications_are_not_state_sources() {
    for fixture in ["notification_permission_prompt", "notification_idle_prompt"] {
        assert_eq!(
            events(fixture),
            vec![mode(PermissionMode::Default)],
            "{fixture}"
        );
    }
}

#[test]
fn stop_ends_the_main_agents_turn() {
    assert_eq!(
        events("stop"),
        vec![mode(PermissionMode::Plan), AgentEvent::TurnEnded]
    );
}

#[test]
fn stop_failure_is_a_failure_of_its_kind() {
    assert_eq!(
        events("stop_failure_rate_limit"),
        vec![
            mode(PermissionMode::Default),
            AgentEvent::Failed {
                kind: FailureKind::RateLimited
            },
        ]
    );
}

#[test]
fn subagents_start_and_finish_without_ending_the_turn() {
    let id = SubagentId("a7f3c9e1b2d4".into());
    assert_eq!(
        events("subagent_start"),
        vec![
            mode(PermissionMode::Default),
            AgentEvent::SubagentStarted {
                id: id.clone(),
                agent_type: "Explore".into(),
                description: String::new(),
            },
        ]
    );
    assert_eq!(
        events("subagent_stop"),
        vec![
            mode(PermissionMode::Default),
            AgentEvent::SubagentFinished { id }
        ]
    );
}

#[test]
fn session_end_carries_nothing_beyond_the_process_exit() {
    assert_eq!(events("session_end"), vec![]);
}

#[test]
fn stop_failure_errors_map_onto_failure_kinds() {
    let kind = |error: &str| {
        let payload = format!(r#"{{"hook_event_name":"StopFailure","error":"{error}"}}"#);
        match ClaudeCode::default().map_hook(&payload).unwrap().as_slice() {
            [AgentEvent::Failed { kind }] => kind.clone(),
            other => panic!("{error}: {other:?}"),
        }
    };
    assert_eq!(kind("overloaded"), FailureKind::Server);
    assert_eq!(kind("server_error"), FailureKind::Server);
    assert_eq!(kind("authentication_failed"), FailureKind::Authentication);
    assert_eq!(kind("billing_error"), FailureKind::Billing);
    assert_eq!(kind("invalid_request"), FailureKind::InvalidRequest);
    assert_eq!(kind("max_output_tokens"), FailureKind::MaxOutputTokens);
    assert_eq!(
        kind("something_new"),
        FailureKind::Other("something_new".into())
    );
}

#[test]
fn unknown_hooks_are_ignored_and_garbage_is_an_error() {
    let claude = ClaudeCode::default();
    assert_eq!(
        claude.map_hook(r#"{"hook_event_name":"PreCompact","trigger":"auto"}"#),
        Ok(vec![])
    );
    assert!(claude.map_hook("not json").is_err());
    assert!(claude.map_hook(r#"{"session_id":"x"}"#).is_err());
}

#[test]
fn a_guard_answer_to_proceed_leaves_the_decision_to_claudes_own_permissions() {
    assert_eq!(
        ClaudeCode::default().guard_answer(&GuardAnswer::Proceed),
        None
    );
}

#[test]
fn a_denied_guard_blocks_the_tool_with_the_reason() {
    let answer = GuardAnswer::Deny {
        reason: "Guard: pushing to the Base branch main".into(),
    };
    let output = ClaudeCode::default().guard_answer(&answer).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output).unwrap(),
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": "Guard: pushing to the Base branch main",
            }
        })
    );
}

#[test]
fn stop_inside_a_subagent_does_not_end_the_main_turn() {
    let payload = r#"{"hook_event_name":"Stop","agent_id":"a7f3c9e1b2d4","agent_type":"Explore"}"#;
    assert_eq!(ClaudeCode::default().map_hook(payload), Ok(vec![]));
}

#[test]
fn an_unanswered_guard_falls_back_to_claudes_own_prompt() {
    let output = ClaudeCode::default()
        .guard_answer(&GuardAnswer::Ask)
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&output).unwrap(),
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "ask",
            }
        })
    );
}

#[test]
fn a_fired_quota_auto_resume_puts_the_agent_back_to_work() {
    assert_eq!(
        events("notification_quota_auto_resume_fired"),
        vec![mode(PermissionMode::Default), AgentEvent::PromptSubmitted]
    );
}

#[test]
fn a_stale_or_disabled_quota_auto_resume_leaves_the_state_alone() {
    for fixture in [
        "notification_quota_auto_resume_stale",
        "notification_quota_auto_resume_disabled",
    ] {
        assert_eq!(
            events(fixture),
            vec![mode(PermissionMode::Default)],
            "{fixture}"
        );
    }
}
