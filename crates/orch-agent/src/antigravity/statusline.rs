use orch_core::{AgentEvent, ConversationId};
use serde::Deserialize;

use super::mode_from_name;
use crate::PayloadError;

const STARTING_UP: [&str; 2] = ["authenticating", "initializing"];

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StatusLine {
    conversation_id: Option<String>,
    agent_state: Option<String>,
    tool_confirmation_pending: Option<bool>,
    cycle_mode: Option<String>,
}

pub(super) fn map_tap(payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
    let line: StatusLine =
        serde_json::from_str(payload).map_err(|err| PayloadError(err.to_string()))?;
    let state = line.agent_state.as_deref().unwrap_or_default();
    let conversation = line.conversation_id.filter(|id| !id.is_empty()).map(|id| {
        AgentEvent::ConversationChanged {
            id: ConversationId(id),
        }
    });
    let mode = (!STARTING_UP.contains(&state))
        .then(|| mode_from_name(line.cycle_mode.as_deref()))
        .flatten()
        .map(|mode| AgentEvent::ModeChanged { mode });
    let prompt = match line.tool_confirmation_pending {
        Some(true) => vec![AgentEvent::PermissionRequested],
        _ if state == "idle" => vec![AgentEvent::PermissionCleared, AgentEvent::AwaitingPrompt],
        _ => vec![AgentEvent::PermissionCleared],
    };
    Ok(conversation.into_iter().chain(mode).chain(prompt).collect())
}
