use orch_core::{AgentEvent, FailureKind};
use serde::Deserialize;
use serde_json::Value;

use super::{GUARD_EVENT, guards};
use crate::PayloadError;
use crate::hook_event::HOOK_EVENT_FIELD;

const QUESTION_TOOL: &str = "ask_question";
const FAILED_REASONS: [&str; 2] = ["error", "max_steps_exceeded"];

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct HookPayload {
    #[serde(skip)]
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
    args: Value,
}

fn error(err: serde_json::Error) -> PayloadError {
    PayloadError(err.to_string())
}

fn parse(value: &Value) -> Result<HookPayload, PayloadError> {
    let event = value[HOOK_EVENT_FIELD].as_str().map(String::from);
    let hook = HookPayload::deserialize(value).map_err(error)?;
    Ok(HookPayload { event, ..hook })
}

pub(super) fn json(payload: &str) -> Result<Value, PayloadError> {
    serde_json::from_str(payload).map_err(error)
}

pub(super) fn is_guard_payload(payload: &str) -> bool {
    let Ok(value) = json(payload) else {
        return false;
    };
    parse(&value).is_ok_and(|hook| match hook.event.as_deref() {
        Some(event) => event == GUARD_EVENT,
        None => hook.tool_call.is_some() && value.get("error").is_none(),
    })
}

pub(super) fn map_hook(payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
    map_value(&json(payload)?)
}

pub(super) fn map_value(value: &Value) -> Result<Vec<AgentEvent>, PayloadError> {
    let hook = parse(value)?;
    let tool = || {
        hook.tool_call
            .as_ref()
            .map(|call| call.name.clone())
            .unwrap_or_default()
    };
    let events = match hook.event.as_deref() {
        Some("PreInvocation") => vec![AgentEvent::PromptSubmitted],
        Some(GUARD_EVENT) if tool() == QUESTION_TOOL => vec![AgentEvent::QuestionAsked],
        Some(GUARD_EVENT) => {
            let check = hook
                .tool_call
                .as_ref()
                .and_then(|call| guards::guard_check(&call.name, &call.args));
            std::iter::once(AgentEvent::ToolStarted {
                tool: tool(),
                subagent: None,
            })
            .chain(check)
            .collect()
        }
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
    if FAILED_REASONS.contains(&reason.as_str()) {
        let error = hook.error.as_deref().filter(|error| !error.is_empty());
        let kind = FailureKind::Other(error.unwrap_or(&reason).into());
        vec![AgentEvent::Failed { kind }]
    } else if hook.fully_idle {
        vec![AgentEvent::TurnEnded]
    } else {
        Vec::new()
    }
}
