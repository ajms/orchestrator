use orch_agent::{AgentAdapter, PayloadError, Pi};
use orch_core::{AgentEvent, ConversationId, FailureKind};

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/pi/hooks/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path}: {err}"))
}

fn events(name: &str) -> Vec<AgentEvent> {
    Pi::default()
        .map_hook(&fixture(name))
        .unwrap_or_else(|err| panic!("{name}: {err:?}"))
}

fn conversation(id: &str) -> AgentEvent {
    AgentEvent::ConversationChanged {
        id: ConversationId(id.into()),
    }
}

fn title(title: &str) -> AgentEvent {
    AgentEvent::TitleChanged {
        title: title.into(),
    }
}

fn failed(kind: FailureKind) -> Vec<AgentEvent> {
    vec![AgentEvent::Failed { kind }]
}

#[test]
fn startup_reports_the_conversation_and_an_idle_agent() {
    assert_eq!(
        events("session_start_startup"),
        vec![
            conversation("64c41acb-5d92-4ea2-be28-64cfd0453e2e"),
            AgentEvent::SessionStarted,
        ]
    );
}

#[test]
fn new_moves_the_session_to_the_new_conversation() {
    assert_eq!(
        events("session_start_new"),
        vec![
            conversation("01a18bde-1d3e-718a-bc4b-2e3eccdb0d01"),
            AgentEvent::SessionStarted,
        ]
    );
}

#[test]
fn a_resumed_named_conversation_brings_its_name_as_the_title() {
    assert_eq!(
        events("session_start_resumed_named"),
        vec![
            conversation("64c41acb-5d92-4ea2-be28-64cfd0453e2e"),
            AgentEvent::SessionStarted,
            title("fix login"),
        ]
    );
}

#[test]
fn a_reload_keeps_the_conversation() {
    let reload = r#"{"event":"session_start","reason":"reload","sessionId":"c9c972bf"}"#;
    assert_eq!(
        Pi::default().map_hook(reload),
        Ok(vec![AgentEvent::SessionStarted])
    );
}

#[test]
fn a_prompt_sets_the_agent_working() {
    assert_eq!(events("input"), vec![AgentEvent::PromptSubmitted]);
    assert_eq!(events("agent_start"), vec![AgentEvent::PromptSubmitted]);
}

#[test]
fn tools_are_reported_as_they_start_and_finish() {
    assert_eq!(
        events("tool_call_bash"),
        vec![AgentEvent::ToolStarted {
            tool: "bash".into(),
            subagent: None
        }]
    );
    assert_eq!(
        events("tool_execution_end_bash"),
        vec![AgentEvent::ToolFinished {
            tool: "bash".into(),
            subagent: None
        }]
    );
}

#[test]
fn the_question_tool_needs_input_until_it_is_answered() {
    assert_eq!(
        events("tool_call_question"),
        vec![AgentEvent::QuestionAsked]
    );
    assert_eq!(
        events("tool_execution_end_question"),
        vec![AgentEvent::ToolFinished {
            tool: "question".into(),
            subagent: None
        }]
    );
}

#[test]
fn a_dialog_needs_input_until_it_closes() {
    for opened in ["ui_prompt_start_select", "ui_prompt_start_confirm"] {
        assert_eq!(events(opened), vec![AgentEvent::QuestionAsked], "{opened}");
    }
    assert_eq!(
        events("ui_prompt_end_working"),
        vec![AgentEvent::PromptSubmitted]
    );
}

#[test]
fn a_dialog_closed_at_the_prompt_leaves_the_agent_idle() {
    assert_eq!(
        events("ui_prompt_end_idle"),
        vec![AgentEvent::SessionStarted]
    );
}

#[test]
fn a_settled_prompt_ends_the_turn() {
    assert_eq!(events("agent_settled"), vec![AgentEvent::TurnEnded]);
}

#[test]
fn escape_ends_the_turn_rather_than_failing() {
    assert_eq!(events("agent_settled_aborted"), vec![AgentEvent::TurnEnded]);
}

#[test]
fn a_provider_error_fails_with_the_kind_its_message_names() {
    assert_eq!(
        events("agent_settled_error_rate_limited"),
        failed(FailureKind::RateLimited)
    );
    assert_eq!(
        events("agent_settled_error_authentication"),
        failed(FailureKind::Authentication)
    );
    assert_eq!(
        events("agent_settled_error_server"),
        failed(FailureKind::Server)
    );
    assert_eq!(
        events("agent_settled_error_context"),
        failed(FailureKind::MaxOutputTokens)
    );
    assert_eq!(
        events("agent_settled_error_other"),
        failed(FailureKind::Other("fetch failed".into()))
    );
}

#[test]
fn error_messages_are_matched_loosely() {
    for (message, kind) in [
        ("Rate limit reached for requests", FailureKind::RateLimited),
        ("403 Forbidden", FailureKind::Authentication),
        ("OAuth token expired", FailureKind::Authentication),
        ("503 Service Unavailable", FailureKind::Server),
        ("The model is overloaded", FailureKind::Server),
        ("Output length exceeded", FailureKind::MaxOutputTokens),
        ("context length 512 exceeded", FailureKind::MaxOutputTokens),
        ("HTTP 502 Bad Gateway", FailureKind::Server),
        ("Request failed with status code 503", FailureKind::Server),
        ("error 429 from upstream", FailureKind::RateLimited),
        (
            "tool 501 failed",
            FailureKind::Other("tool 501 failed".into()),
        ),
        (
            "status 5000 tokens",
            FailureKind::Other("status 5000 tokens".into()),
        ),
    ] {
        let payload = serde_json::json!({
            "event": "agent_settled",
            "aborted": false,
            "stopReason": "error",
            "errorMessage": message,
        });
        assert_eq!(
            Pi::default().map_hook(&payload.to_string()),
            Ok(failed(kind)),
            "{message}"
        );
    }
}

#[test]
fn a_rename_becomes_the_title_and_clearing_it_brings_the_slug_back() {
    assert_eq!(
        events("session_info_changed_named"),
        vec![title("fix login")]
    );
    assert_eq!(events("session_info_changed_cleared"), vec![title("")]);
}

#[test]
fn unknown_events_map_to_nothing() {
    assert_eq!(
        Pi::default().map_hook(r#"{"event":"turn_end","turnIndex":2}"#),
        Ok(vec![])
    );
}

#[test]
fn a_payload_that_isnt_an_event_is_refused() {
    assert!(matches!(
        Pi::default().map_hook("not json"),
        Err(PayloadError(_))
    ));
}
