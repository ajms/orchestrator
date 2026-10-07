use orch_core::{AgentEvent, ConversationId};
use serde::Deserialize;

use super::mode_from_name;
use super::usage::Usage;
use crate::PayloadError;

const STARTING_UP: [&str; 2] = ["authenticating", "initializing"];

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StatusLine {
    conversation_id: Option<String>,
    agent_state: Option<String>,
    tool_confirmation_pending: Option<bool>,
    cycle_mode: Option<String>,
    #[serde(flatten)]
    usage: Usage,
}

pub(super) fn map_tap(payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
    let line: StatusLine =
        serde_json::from_str(payload).map_err(|err| PayloadError(err.to_string()))?;
    let state = line.agent_state.as_deref().unwrap_or_default();
    let conversation_id = line
        .conversation_id
        .filter(|id| !id.is_empty())
        .map(ConversationId);
    let conversation = conversation_id
        .clone()
        .map(|id| AgentEvent::ConversationChanged { id });
    let started = !STARTING_UP.contains(&state);
    let usage = started.then(|| AgentEvent::UsageSample(line.usage.sample(conversation_id)));
    let mode = started
        .then(|| mode_from_name(line.cycle_mode.as_deref()))
        .flatten()
        .map(|mode| AgentEvent::ModeChanged { mode });
    let prompt = match line.tool_confirmation_pending {
        Some(true) => vec![AgentEvent::PermissionRequested],
        _ if state == "idle" && conversation.is_some() => {
            vec![AgentEvent::PermissionCleared, AgentEvent::AwaitingPrompt]
        }
        _ => vec![AgentEvent::PermissionCleared],
    };
    Ok(conversation
        .into_iter()
        .chain(usage)
        .chain(mode)
        .chain(prompt)
        .collect())
}
