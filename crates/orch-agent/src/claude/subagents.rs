use std::collections::HashMap;
use std::path::{Path, PathBuf};

use orch_core::{SubagentId, TranscriptEntry};
use serde::Deserialize;
use serde_json::Value;

use super::lines::FollowedLines;
use crate::{SubagentTranscripts, TranscriptRead, TranscriptReader};

const KEY_ARGUMENTS: [&str; 9] = [
    "command",
    "file_path",
    "notebook_path",
    "pattern",
    "url",
    "query",
    "skill",
    "description",
    "prompt",
];

#[derive(Debug, Default)]
pub(super) struct ClaudeSubagentTranscripts {
    paths: HashMap<String, PathBuf>,
}

#[derive(Deserialize)]
struct HookPaths {
    transcript_path: Option<PathBuf>,
    agent_id: Option<String>,
    agent_transcript_path: Option<PathBuf>,
}

impl SubagentTranscripts for ClaudeSubagentTranscripts {
    fn follow(&mut self, payload: &str) {
        let Ok(followed) = serde_json::from_str::<HookPaths>(payload) else {
            return;
        };
        let Some(id) = followed.agent_id else {
            return;
        };
        if let Some(path) = followed.agent_transcript_path {
            self.paths.insert(id, path);
        } else if let Some(session) = followed.transcript_path
            && !self.paths.contains_key(&id)
        {
            let path = session
                .with_extension("")
                .join("subagents")
                .join(format!("agent-{id}.jsonl"));
            self.paths.insert(id, path);
        }
    }

    fn locate(&self, subagent: &SubagentId) -> Option<PathBuf> {
        self.paths.get(subagent.as_str()).cloned()
    }

    fn reader(&self) -> Box<dyn TranscriptReader> {
        Box::new(ClaudeTranscriptReader::default())
    }
}

#[derive(Debug, Default)]
struct ClaudeTranscriptReader {
    lines: FollowedLines,
}

impl TranscriptReader for ClaudeTranscriptReader {
    fn read(&mut self, path: &Path) -> TranscriptRead {
        let Some(read) = self.lines.read(path) else {
            return TranscriptRead::default();
        };
        TranscriptRead {
            reset: read.reset,
            entries: read.lines.iter().flat_map(|line| entries(line)).collect(),
        }
    }
}

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "isMeta", default)]
    is_meta: bool,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    content: Content,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Content {
    Text(String),
    Blocks(Vec<Block>),
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Block {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: Option<Content>,
        #[serde(default)]
        is_error: bool,
    },
    #[serde(other)]
    Other,
}

fn entries(line: &[u8]) -> Vec<TranscriptEntry> {
    let Ok(line) = serde_json::from_slice::<Line>(line) else {
        return Vec::new();
    };
    let Some(message) = line.message.filter(|_| !line.is_meta) else {
        return Vec::new();
    };
    let from_user = match line.kind.as_str() {
        "user" => true,
        "assistant" => false,
        _ => return Vec::new(),
    };
    let blocks = match message.content {
        Content::Text(text) if from_user => return vec![TranscriptEntry::Prompt { text }],
        Content::Text(text) => return vec![TranscriptEntry::Text { text }],
        Content::Blocks(blocks) => blocks,
    };
    let answers_tools = blocks
        .iter()
        .any(|block| matches!(block, Block::ToolResult { .. }));
    blocks
        .into_iter()
        .filter_map(|block| match block {
            Block::Text { .. } if answers_tools => None,
            Block::Text { text } if from_user => Some(TranscriptEntry::Prompt { text }),
            Block::Text { text } => Some(TranscriptEntry::Text { text }),
            Block::ToolUse { id, name, input } => Some(TranscriptEntry::ToolCall {
                id,
                argument: key_argument(&input),
                tool: name,
            }),
            Block::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => Some(TranscriptEntry::ToolResult {
                id: tool_use_id,
                text: content.map(Content::into_text).unwrap_or_default(),
                error: is_error,
            }),
            Block::Other => None,
        })
        .collect()
}

impl Content {
    fn into_text(self) -> String {
        match self {
            Content::Text(text) => text,
            Content::Blocks(blocks) => blocks
                .into_iter()
                .filter_map(|block| match block {
                    Block::Text { text } => Some(text),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }
}

fn key_argument(input: &Value) -> Option<String> {
    KEY_ARGUMENTS
        .iter()
        .find_map(|key| input.get(key)?.as_str())
        .map(String::from)
}
