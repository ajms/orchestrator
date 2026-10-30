use orch_core::{AgentEvent, ConversationId, FailureKind};
use serde::Deserialize;

use crate::PayloadError;

const QUESTION_TOOL: &str = "question";
const STATUS_WORDS: [&str; 4] = ["status", "code", "error", "http"];

#[derive(Debug, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum Hook {
    #[serde(rename_all = "camelCase")]
    SessionStart {
        reason: String,
        session_id: Option<String>,
        name: Option<String>,
    },
    SessionInfoChanged {
        name: Option<String>,
    },
    Input,
    AgentStart,
    #[serde(rename_all = "camelCase")]
    ToolCall {
        tool_name: String,
    },
    #[serde(rename_all = "camelCase")]
    ToolExecutionEnd {
        tool_name: String,
    },
    UiPromptStart,
    UiPromptEnd {
        #[serde(default)]
        idle: bool,
    },
    #[serde(rename_all = "camelCase")]
    AgentSettled {
        aborted: bool,
        stop_reason: Option<String>,
        error_message: Option<String>,
    },
    #[serde(other)]
    Other,
}

pub(super) fn map_hook(payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
    let hook: Hook = serde_json::from_str(payload).map_err(|err| PayloadError(err.to_string()))?;
    Ok(hook.events())
}

impl Hook {
    fn events(self) -> Vec<AgentEvent> {
        match self {
            Hook::SessionStart {
                reason,
                session_id,
                name,
            } => {
                let conversation = session_id.filter(|_| reason != "reload").map(|id| {
                    AgentEvent::ConversationChanged {
                        id: ConversationId(id),
                    }
                });
                let title = name
                    .filter(|name| !name.is_empty())
                    .map(|title| AgentEvent::TitleChanged { title });
                conversation
                    .into_iter()
                    .chain([AgentEvent::SessionStarted])
                    .chain(title)
                    .collect()
            }
            Hook::SessionInfoChanged { name } => vec![AgentEvent::TitleChanged {
                title: name.unwrap_or_default(),
            }],
            Hook::Input | Hook::AgentStart => vec![AgentEvent::PromptSubmitted],
            Hook::ToolCall { tool_name } if tool_name == QUESTION_TOOL => {
                vec![AgentEvent::QuestionAsked]
            }
            Hook::ToolCall { tool_name } => vec![AgentEvent::ToolStarted {
                tool: tool_name,
                subagent: None,
            }],
            Hook::ToolExecutionEnd { tool_name } => vec![AgentEvent::ToolFinished {
                tool: tool_name,
                subagent: None,
            }],
            Hook::UiPromptStart => vec![AgentEvent::QuestionAsked],
            // Back at pi's prompt: a dialog a command opened while no prompt ran.
            Hook::UiPromptEnd { idle: true } => vec![AgentEvent::SessionStarted],
            Hook::UiPromptEnd { idle: false } => vec![AgentEvent::PromptSubmitted],
            Hook::AgentSettled {
                aborted: false,
                stop_reason: Some(reason),
                error_message,
            } if reason == "error" => vec![AgentEvent::Failed {
                kind: failure_kind(error_message.as_deref().unwrap_or("unknown")),
            }],
            Hook::AgentSettled { .. } => vec![AgentEvent::TurnEnded],
            Hook::Other => vec![],
        }
    }
}

fn failure_kind(message: &str) -> FailureKind {
    let lower = message.to_lowercase();
    let says = |words: &[&str]| words.iter().any(|word| lower.contains(word));
    let code = status_code(&lower);
    let status = |matches: fn(&str) -> bool| code.is_some_and(matches);
    if status(|code| code == "429") || says(&["rate limit", "rate_limit", "too many requests"]) {
        FailureKind::RateLimited
    } else if status(|code| code == "401" || code == "403")
        || says(&["auth", "unauthorized", "forbidden"])
    {
        FailureKind::Authentication
    } else if status(|code| code.starts_with('5')) || says(&["overloaded"]) {
        FailureKind::Server
    } else if says(&["context", "length"]) {
        FailureKind::MaxOutputTokens
    } else {
        FailureKind::Other(message.into())
    }
}

fn status_code(message: &str) -> Option<&str> {
    let words: Vec<&str> = message
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    words.iter().enumerate().find_map(|(at, word)| {
        let numbered = word.len() == 3 && word.bytes().all(|b| b.is_ascii_digit());
        let named = at == 0 || STATUS_WORDS.contains(&words[at - 1]);
        (numbered && named).then_some(*word)
    })
}
