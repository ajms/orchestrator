use orch_core::{AgentEvent, ConversationId, FailureKind, SubagentId};
use serde::Deserialize;

use crate::PayloadError;

const QUESTION_TOOL: &str = "question";
pub(super) const SUBAGENT_TOOL: &str = "subagent";
const LABEL_CHARS: usize = 60;
const STATUS_WORDS: [&str; 4] = ["status", "code", "error", "http"];

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StartReason {
    Reload,
    #[serde(other)]
    Other,
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StopReason {
    Error,
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum Hook {
    #[serde(rename_all = "camelCase")]
    SessionStart {
        reason: StartReason,
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
        tool_call_id: Option<String>,
        agent: Option<String>,
        task_name: Option<String>,
        task: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    ToolExecutionEnd {
        tool_name: String,
        tool_call_id: Option<String>,
    },
    UiPromptStart,
    UiPromptEnd {
        #[serde(default)]
        idle: bool,
    },
    #[serde(rename_all = "camelCase")]
    AgentSettled {
        aborted: bool,
        stop_reason: Option<StopReason>,
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
                let conversation = session_id
                    .filter(|_| reason != StartReason::Reload)
                    .map(|id| AgentEvent::ConversationChanged {
                        id: ConversationId(id),
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
            Hook::ToolCall { tool_name, .. } if tool_name == QUESTION_TOOL => {
                vec![AgentEvent::QuestionAsked]
            }
            Hook::ToolCall {
                tool_name,
                tool_call_id,
                agent,
                task_name,
                task,
            } => {
                let started =
                    subagent(&tool_name, tool_call_id).map(|id| AgentEvent::SubagentStarted {
                        id,
                        agent_type: given(agent).unwrap_or_else(|| SUBAGENT_TOOL.into()),
                        description: given(task_name)
                            .or_else(|| given(task).map(|task| cut_short(&task)))
                            .unwrap_or_default(),
                    });
                std::iter::once(AgentEvent::ToolStarted {
                    tool: tool_name,
                    subagent: None,
                })
                .chain(started)
                .collect()
            }
            Hook::ToolExecutionEnd {
                tool_name,
                tool_call_id,
            } => {
                let finished = subagent(&tool_name, tool_call_id)
                    .map(|id| AgentEvent::SubagentFinished { id });
                std::iter::once(AgentEvent::ToolFinished {
                    tool: tool_name,
                    subagent: None,
                })
                .chain(finished)
                .collect()
            }
            Hook::UiPromptStart => vec![AgentEvent::QuestionAsked],
            // Back at pi's prompt: a dialog a command opened while no prompt ran.
            Hook::UiPromptEnd { idle: true } => vec![AgentEvent::SessionStarted],
            Hook::UiPromptEnd { idle: false } => vec![AgentEvent::PromptSubmitted],
            Hook::AgentSettled {
                aborted: false,
                stop_reason: Some(StopReason::Error),
                error_message,
            } => vec![AgentEvent::Failed {
                kind: failure_kind(error_message.as_deref().unwrap_or("unknown")),
            }],
            Hook::AgentSettled { .. } => vec![AgentEvent::TurnEnded],
            Hook::Other => vec![],
        }
    }
}

fn subagent(tool: &str, call: Option<String>) -> Option<SubagentId> {
    call.filter(|_| tool == SUBAGENT_TOOL).map(SubagentId)
}

fn given(label: Option<String>) -> Option<String> {
    label.filter(|label| !label.trim().is_empty())
}

fn cut_short(text: &str) -> String {
    if text.chars().count() <= LABEL_CHARS {
        return text.into();
    }
    let head: String = text.chars().take(LABEL_CHARS).collect();
    let words = head
        .rsplit_once(' ')
        .map_or(head.as_str(), |(words, _)| words);
    format!("{}…", words.trim_end())
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
