use orch_core::{
    AgentEvent, ConversationId, FailureKind, SubagentId, SubagentMerge, SubagentOutcome,
    SubagentUsage, SubagentWorktree,
};
use serde::Deserialize;

use crate::bounded::{NAME_CHARS, REASON_CHARS, SUMMARY_CHARS, bounded, bounded_line};
use crate::{PayloadError, WorktreeRequest};

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
        #[serde(flatten)]
        outcome: Box<SubagentEnd>,
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

/// What orch's extension forwards from a `subagent` call's final result
/// (the user's extension's `SingleResult`). It comes from inside the Sandbox.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SubagentEnd {
    is_error: bool,
    exit_code: Option<i64>,
    stop_reason: Option<String>,
    error_message: Option<String>,
    model: Option<String>,
    usage: Option<SubagentEndUsage>,
    text: Option<String>,
    worktree_path: Option<String>,
    worktree_branch: Option<String>,
    worktree_cleanup: Option<SubagentEndCleanup>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SubagentEndUsage {
    input: Option<f64>,
    output: Option<f64>,
    cost: Option<f64>,
    turns: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SubagentEndCleanup {
    merged: Option<bool>,
    skipped: Option<bool>,
    message: Option<String>,
    preserved_branch: Option<String>,
}

impl SubagentEnd {
    fn into_outcome(self) -> SubagentOutcome {
        let stop_reason = self
            .stop_reason
            .as_deref()
            .and_then(|r| bounded_line(r, NAME_CHARS));
        let stopped_badly = matches!(stop_reason.as_deref(), Some("error" | "aborted"));
        let exit_code = self.exit_code.filter(|code| *code != 0);
        let errored = self.is_error || exit_code.is_some() || stopped_badly;
        let failure = errored.then(|| {
            self.error_message
                .as_deref()
                .and_then(|message| bounded_line(message, REASON_CHARS))
                .or_else(|| stop_reason.filter(|_| stopped_badly))
                .or_else(|| exit_code.map(|code| format!("exit code {code}")))
                .unwrap_or_else(|| "failed".into())
        });
        let usage = self.usage.map(|usage| SubagentUsage {
            input_tokens: non_negative_u64(usage.input),
            output_tokens: non_negative_u64(usage.output),
            cost_usd: usage.cost.filter(|cost| cost.is_finite() && *cost >= 0.0),
            turns: non_negative_u64(usage.turns).and_then(|turns| u32::try_from(turns).ok()),
        });
        let cleanup = self.worktree_cleanup.unwrap_or_default();
        let worktree = SubagentWorktree {
            path: self
                .worktree_path
                .as_deref()
                .and_then(|p| bounded_line(p, REASON_CHARS)),
            branch: self
                .worktree_branch
                .as_deref()
                .and_then(|b| bounded_line(b, NAME_CHARS)),
            merge: match (cleanup.merged, cleanup.skipped) {
                (Some(true), _) => Some(SubagentMerge::Merged),
                (_, Some(true)) => Some(SubagentMerge::NothingToMerge),
                (Some(false), _) => Some(SubagentMerge::NotMerged),
                _ => None,
            },
            message: cleanup
                .message
                .as_deref()
                .and_then(|m| bounded_line(m, REASON_CHARS)),
            preserved_branch: cleanup
                .preserved_branch
                .as_deref()
                .and_then(|b| bounded_line(b, NAME_CHARS)),
        };
        // A failed call's text restates its failure; only other text is kept.
        let summary = self
            .text
            .as_deref()
            .and_then(|text| bounded(text, SUMMARY_CHARS))
            .filter(|text| {
                !failure
                    .as_ref()
                    .is_some_and(|failure| text.contains(failure.as_str()))
            });
        SubagentOutcome {
            errored,
            summary,
            failure,
            model: self
                .model
                .as_deref()
                .and_then(|m| bounded_line(m, NAME_CHARS)),
            usage,
            worktree: (worktree != SubagentWorktree::default()).then(|| Box::new(worktree)),
        }
    }
}

fn non_negative_u64(value: Option<f64>) -> Option<u64> {
    value
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value as u64)
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
                let started = subagent_of_call(&tool_name, tool_call_id).map(|id| {
                    AgentEvent::SubagentStarted {
                        id,
                        agent_type: given(agent).unwrap_or_else(|| SUBAGENT_TOOL.into()),
                        description: given(task_name)
                            .or_else(|| given(task).map(|task| cut_short(&task)))
                            .unwrap_or_default(),
                    }
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
                outcome,
            } => {
                let finished = subagent_of_call(&tool_name, tool_call_id).map(|id| {
                    AgentEvent::SubagentFinished {
                        id,
                        outcome: outcome.into_outcome(),
                    }
                });
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

fn subagent_of_call(tool: &str, call: Option<String>) -> Option<SubagentId> {
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

#[derive(Debug, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum WorktreeHook {
    WorktreeCreate { name: String },
    WorktreeRemove { path: std::path::PathBuf },
}

pub(super) fn worktree_request(payload: &str) -> Option<WorktreeRequest> {
    match serde_json::from_str(payload).ok()? {
        WorktreeHook::WorktreeCreate { name } => Some(WorktreeRequest::Create { name }),
        WorktreeHook::WorktreeRemove { path } => Some(WorktreeRequest::Remove { path }),
    }
}
