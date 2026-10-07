use std::path::PathBuf;

use orch_core::{AgentEvent, ConversationId, FailureKind, SubagentId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::mode_from_name;
use crate::{PayloadError, WorktreeRequest};

const QUESTION_TOOL: &str = "AskUserQuestion";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum HookEvent {
    SessionStart,
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    PermissionRequest,
    PermissionDenied,
    Notification,
    Stop,
    StopFailure,
    SubagentStart,
    SubagentStop,
    WorktreeCreate,
    WorktreeRemove,
    #[serde(other)]
    Other,
}

impl HookEvent {
    pub(super) const OBSERVED: [HookEvent; 12] = [
        HookEvent::SessionStart,
        HookEvent::UserPromptSubmit,
        HookEvent::PreToolUse,
        HookEvent::PostToolUse,
        HookEvent::PostToolUseFailure,
        HookEvent::PermissionRequest,
        HookEvent::PermissionDenied,
        HookEvent::Notification,
        HookEvent::Stop,
        HookEvent::StopFailure,
        HookEvent::SubagentStart,
        HookEvent::SubagentStop,
    ];

    pub(super) fn name(self) -> String {
        match serde_json::to_value(self) {
            Ok(Value::String(name)) => name,
            _ => unreachable!("hook events serialize as their names"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct HookPayload {
    hook_event_name: HookEvent,
    session_id: Option<String>,
    cwd: Option<String>,
    permission_mode: Option<String>,
    source: Option<String>,
    tool_name: Option<String>,
    tool_input: Option<Value>,
    agent_id: Option<String>,
    agent_type: Option<String>,
    notification_type: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct HookName {
    hook_event_name: HookEvent,
}

pub(super) fn event_name(payload: &str) -> Option<HookEvent> {
    serde_json::from_str::<HookName>(payload)
        .ok()
        .map(|hook| hook.hook_event_name)
}

#[derive(Deserialize)]
struct WorktreePayload {
    hook_event_name: HookEvent,
    name: Option<String>,
    worktree_path: Option<PathBuf>,
}

pub(super) fn worktree_request(payload: &str) -> Option<WorktreeRequest> {
    let hook: WorktreePayload = serde_json::from_str(payload).ok()?;
    match hook.hook_event_name {
        HookEvent::WorktreeCreate => hook.name.map(|name| WorktreeRequest::Create { name }),
        HookEvent::WorktreeRemove => hook
            .worktree_path
            .map(|path| WorktreeRequest::Remove { path }),
        _ => None,
    }
}

pub(super) fn map_hook(payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
    let hook: HookPayload =
        serde_json::from_str(payload).map_err(|err| PayloadError(err.to_string()))?;
    let mode = hook
        .permission_mode
        .as_deref()
        .and_then(mode_from_name)
        .map(|mode| AgentEvent::ModeChanged { mode });
    Ok(mode.into_iter().chain(hook.events()).collect())
}

impl HookPayload {
    fn events(self) -> Vec<AgentEvent> {
        use HookEvent as H;
        let tool = self.tool_name.clone().unwrap_or_default();
        let subagent = self.agent_id.clone().map(SubagentId);
        let asks_user = tool == QUESTION_TOOL;
        match self.hook_event_name {
            H::SessionStart => {
                let conversation = self.session_id.map(|id| AgentEvent::ConversationChanged {
                    id: ConversationId(id),
                });
                let started = (self.source.as_deref() != Some("compact"))
                    .then_some(AgentEvent::SessionStarted);
                conversation.into_iter().chain(started).collect()
            }
            H::UserPromptSubmit => vec![AgentEvent::PromptSubmitted],
            H::PreToolUse | H::PermissionRequest if asks_user => vec![AgentEvent::QuestionAsked],
            H::PreToolUse => {
                let input_json = self.tool_input.unwrap_or(Value::Null).to_string();
                vec![
                    AgentEvent::ToolStarted {
                        tool: tool.clone(),
                        subagent,
                    },
                    AgentEvent::GuardCheck {
                        tool,
                        input_json,
                        cwd: self.cwd,
                    },
                ]
            }
            H::PostToolUse | H::PostToolUseFailure => {
                vec![AgentEvent::ToolFinished { tool, subagent }]
            }
            H::PermissionRequest => vec![AgentEvent::PermissionRequested],
            H::PermissionDenied => vec![AgentEvent::PermissionDenied],
            H::Notification => match self.notification_type.as_deref() {
                Some("elicitation_dialog" | "elicitation_url_dialog" | "agent_needs_input") => {
                    vec![AgentEvent::QuestionAsked]
                }
                Some("quota_auto_resume_fired") => vec![AgentEvent::PromptSubmitted],
                _ => vec![],
            },
            H::Stop if subagent.is_none() => vec![AgentEvent::TurnEnded],
            H::Stop | H::WorktreeCreate | H::WorktreeRemove | H::Other => vec![],
            H::StopFailure => vec![AgentEvent::Failed {
                kind: failure_kind(self.error.as_deref().unwrap_or("unknown")),
            }],
            // Known limitation: the description lives on the parent's Task/Agent PreToolUse, which a pure mapper cannot correlate.
            H::SubagentStart => subagent
                .map(|id| AgentEvent::SubagentStarted {
                    id,
                    agent_type: self.agent_type.unwrap_or_default(),
                    description: String::new(),
                })
                .into_iter()
                .collect(),
            H::SubagentStop => subagent
                .map(|id| AgentEvent::SubagentFinished { id })
                .into_iter()
                .collect(),
        }
    }
}

fn failure_kind(error: &str) -> FailureKind {
    match error {
        "rate_limit" => FailureKind::RateLimited,
        "authentication_failed" | "oauth_org_not_allowed" | "cloud_credential_error" => {
            FailureKind::Authentication
        }
        "billing_error" | "account_on_hold" => FailureKind::Billing,
        "invalid_request" | "model_not_found" => FailureKind::InvalidRequest,
        "server_error" | "overloaded" => FailureKind::Server,
        "max_output_tokens" => FailureKind::MaxOutputTokens,
        other => FailureKind::Other(other.into()),
    }
}
