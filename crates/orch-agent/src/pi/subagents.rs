use std::collections::HashMap;
use std::path::{Path, PathBuf};

use orch_core::{ConversationId, SubagentId, TranscriptEntry};
use serde::Deserialize;
use serde_json::Value;

use super::hooks::SUBAGENT_TOOL;
use crate::key_argument::key_argument;
use crate::lines::FollowedLines;
use crate::{SubagentTranscripts, TranscriptRead, TranscriptReader};

const DIR: &str = "subagents";
const MAX_ID_BYTES: usize = 200;

#[derive(Debug)]
pub(super) struct PiSubagentTranscripts {
    profile: PathBuf,
    conversations: HashMap<SubagentId, ConversationId>,
}

impl PiSubagentTranscripts {
    pub(super) fn new(profile: PathBuf) -> Self {
        Self {
            profile,
            conversations: HashMap::new(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Call {
    event: String,
    tool_name: Option<String>,
    tool_call_id: Option<String>,
    session_id: Option<ConversationId>,
}

fn plain_name(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_BYTES
        && !id.starts_with('.')
        && !id.contains(['/', '\\', '\0'])
}

impl SubagentTranscripts for PiSubagentTranscripts {
    fn follow(&mut self, payload: &str) {
        let Ok(call) = serde_json::from_str::<Call>(payload) else {
            return;
        };
        if call.event != "tool_call" || call.tool_name.as_deref() != Some(SUBAGENT_TOOL) {
            return;
        }
        if let (Some(id), Some(conversation)) = (call.tool_call_id, call.session_id)
            && plain_name(&id)
            && plain_name(conversation.as_str())
        {
            self.conversations.insert(SubagentId(id), conversation);
        }
    }

    fn locate(&self, subagent: &SubagentId) -> Option<PathBuf> {
        let conversation = self.conversations.get(subagent)?;
        Some(
            self.profile
                .join(DIR)
                .join(conversation.as_str())
                .join(format!("{}.jsonl", subagent.as_str())),
        )
    }

    fn describe(&self, _subagent: &SubagentId) -> Option<String> {
        None
    }

    fn reader(&self) -> Box<dyn TranscriptReader> {
        Box::new(PiTranscriptReader {
            profile: self.profile.clone(),
            lines: FollowedLines::default(),
        })
    }
}

#[derive(Debug)]
struct PiTranscriptReader {
    profile: PathBuf,
    lines: FollowedLines,
}

impl TranscriptReader for PiTranscriptReader {
    fn read(&mut self, path: &Path) -> TranscriptRead {
        let Ok(profile) = std::fs::canonicalize(&self.profile) else {
            return TranscriptRead::default();
        };
        let Some(read) = self.lines.read_within(&profile.join(DIR), path) else {
            return TranscriptRead::default();
        };
        TranscriptRead {
            reset: read.reset,
            entries: read
                .lines
                .iter()
                .flat_map(|line| message_entries(line))
                .collect(),
        }
    }
}

#[derive(Deserialize)]
#[serde(
    tag = "role",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Message {
    User {
        content: Content,
    },
    Assistant {
        content: Vec<Block>,
    },
    ToolResult {
        tool_call_id: String,
        #[serde(default)]
        content: Vec<Block>,
        #[serde(default)]
        is_error: bool,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Content {
    Text(String),
    Blocks(Vec<Block>),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Block {
    Text {
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        #[serde(default)]
        arguments: Value,
    },
    #[serde(other)]
    Other,
}

fn message_entries(line: &[u8]) -> Vec<TranscriptEntry> {
    let Ok(message) = serde_json::from_slice::<Message>(line) else {
        return Vec::new();
    };
    match message {
        Message::User { content } => {
            let text = match content {
                Content::Text(text) => text,
                Content::Blocks(blocks) => text_of(blocks),
            };
            (!text.is_empty())
                .then_some(TranscriptEntry::Prompt { text })
                .into_iter()
                .collect()
        }
        Message::Assistant { content } => content
            .into_iter()
            .filter_map(|block| match block {
                Block::Text { text } => Some(TranscriptEntry::Text { text }),
                Block::ToolCall {
                    id,
                    name,
                    arguments,
                } => Some(TranscriptEntry::ToolCall {
                    id,
                    argument: key_argument(&arguments)
                        .or_else(|| arguments.get("path")?.as_str().map(String::from)),
                    tool: name,
                }),
                Block::Other => None,
            })
            .collect(),
        Message::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => vec![TranscriptEntry::ToolResult {
            id: tool_call_id,
            text: text_of(content),
            error: is_error,
        }],
        Message::Other => Vec::new(),
    }
}

fn text_of(blocks: Vec<Block>) -> String {
    blocks
        .into_iter()
        .filter_map(|block| match block {
            Block::Text { text } => Some(text),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
