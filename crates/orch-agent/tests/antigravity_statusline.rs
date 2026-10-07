use orch_agent::{AgentAdapter, Antigravity};
use orch_core::{AgentEvent, ConversationId, PermissionMode};

const CONVERSATION: &str = "3c1e9a40-7d52-4b8e-a6f1-2d9b0c4e7a13";

fn events(name: &str) -> Vec<AgentEvent> {
    let path = format!(
        "{}/tests/fixtures/antigravity/statusline/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let payload = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path}: {err}"));
    Antigravity::default()
        .map_tap(&payload)
        .unwrap_or_else(|err| panic!("{name}: {err:?}"))
}

fn conversation() -> AgentEvent {
    AgentEvent::ConversationChanged {
        id: ConversationId(CONVERSATION.into()),
    }
}

fn mode(mode: PermissionMode) -> AgentEvent {
    AgentEvent::ModeChanged { mode }
}

#[test]
fn the_trust_screen_needs_input_before_any_conversation_or_mode_exists() {
    assert_eq!(events("trust_screen"), [AgentEvent::PermissionRequested]);
}

#[test]
fn an_idle_line_reports_the_conversation_the_default_mode_and_readiness() {
    assert_eq!(
        events("idle"),
        [
            conversation(),
            mode(PermissionMode::Default),
            AgentEvent::PermissionCleared,
            AgentEvent::Ready,
        ]
    );
}

#[test]
fn a_pending_tool_confirmation_needs_input_in_the_cycled_mode() {
    assert_eq!(
        events("tool_confirmation_accept_edits"),
        [
            conversation(),
            mode(PermissionMode::AcceptEdits),
            AgentEvent::PermissionRequested,
        ]
    );
}

#[test]
fn a_working_line_clears_a_permission_prompt_and_tracks_plan_mode() {
    assert_eq!(
        events("working_plan"),
        [
            conversation(),
            mode(PermissionMode::Plan),
            AgentEvent::PermissionCleared,
        ]
    );
}

#[test]
fn garbage_statusline_input_is_an_error() {
    assert!(Antigravity::default().map_tap("not json").is_err());
}
