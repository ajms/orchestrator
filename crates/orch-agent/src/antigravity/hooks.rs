use orch_core::{AgentEvent, FailureKind};
use serde::Deserialize;

use crate::PayloadError;

const QUESTION_TOOL: &str = "ask_question";
const GUARD_EVENT: &str = "PreToolUse";

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct HookPayload {
    #[serde(rename = "orch_hook_event")]
    event: Option<String>,
    tool_call: Option<ToolCall>,
    termination_reason: Option<String>,
    error: Option<String>,
    fully_idle: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ToolCall {
    name: String,
}

fn parse(payload: &str) -> Result<HookPayload, PayloadError> {
    serde_json::from_str(payload).map_err(|err| PayloadError(err.to_string()))
}

pub(super) fn is_guard_payload(payload: &str) -> bool {
    parse(payload).is_ok_and(|hook| match hook.event.as_deref() {
        Some(event) => event == GUARD_EVENT,
        None => hook.tool_call.is_some(),
    })
}

pub(super) fn map_hook(payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
    let hook = parse(payload)?;
    let tool = || {
        hook.tool_call
            .as_ref()
            .map(|call| call.name.clone())
            .unwrap_or_default()
    };
    let events = match hook.event.as_deref() {
        Some("PreInvocation" | "PostInvocation") => vec![AgentEvent::PromptSubmitted],
        Some("PreToolUse") if tool() == QUESTION_TOOL => vec![AgentEvent::QuestionAsked],
        Some("PreToolUse") => vec![AgentEvent::ToolStarted {
            tool: tool(),
            subagent: None,
        }],
        Some("PostToolUse") => vec![AgentEvent::ToolFinished {
            tool: tool(),
            subagent: None,
        }],
        Some("Stop") => stop(&hook),
        _ => Vec::new(),
    };
    Ok(events)
}

fn stop(hook: &HookPayload) -> Vec<AgentEvent> {
    let reason = hook
        .termination_reason
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let reason = reason.trim_start_matches("termination_reason_");
    if reason == "error" || reason.starts_with("max_") {
        let error = hook.error.as_deref().filter(|error| !error.is_empty());
        let kind = FailureKind::Other(error.unwrap_or(reason).into());
        return vec![AgentEvent::Failed { kind }];
    }
    match hook.fully_idle {
        true => vec![AgentEvent::TurnEnded],
        false => Vec::new(),
    }
}
