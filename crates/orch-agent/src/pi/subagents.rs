use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use nix::fcntl::OFlag;
use orch_core::{SubagentId, TranscriptEntry};
use serde::Deserialize;
use serde_json::Value;

use super::hooks::SUBAGENT_TOOL;
use crate::key_argument::key_argument;
use crate::{SubagentTranscripts, TranscriptRead, TranscriptReader};

const DIR: &str = "subagents";
const MAX_ID_BYTES: usize = 200;
const READ_BUDGET: u64 = 8 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct PiSubagentTranscripts {
    profile: PathBuf,
    conversations: HashMap<String, String>,
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
    session_id: Option<String>,
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
            && plain_name(&conversation)
        {
            self.conversations.insert(id, conversation);
        }
    }

    fn locate(&self, subagent: &SubagentId) -> Option<PathBuf> {
        let conversation = self.conversations.get(subagent.as_str())?;
        Some(
            self.profile
                .join(DIR)
                .join(conversation)
                .join(format!("{}.jsonl", subagent.as_str())),
        )
    }

    fn describe(&self, _subagent: &SubagentId) -> Option<String> {
        None
    }

    fn reader(&self) -> Box<dyn TranscriptReader> {
        Box::new(PiTranscriptReader {
            profile: self.profile.clone(),
            followed: None,
            skipping: false,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Followed {
    path: PathBuf,
    inode: (u64, u64),
    offset: u64,
}

#[derive(Debug)]
struct PiTranscriptReader {
    profile: PathBuf,
    followed: Option<Followed>,
    skipping: bool,
}

impl TranscriptReader for PiTranscriptReader {
    fn read(&mut self, path: &Path) -> TranscriptRead {
        let Some(mut file) = open_within(&self.profile, path) else {
            return TranscriptRead::default();
        };
        let Ok(meta) = file.metadata() else {
            return TranscriptRead::default();
        };
        let inode = (meta.dev(), meta.ino());
        let reset = !self.followed.as_ref().is_some_and(|followed| {
            followed.path == path && followed.inode == inode && followed.offset <= meta.len()
        });
        let offset = match &self.followed {
            Some(followed) if !reset => followed.offset,
            _ => 0,
        };
        if reset {
            self.skipping = false;
        }
        let followed = self.followed.insert(Followed {
            path: path.to_path_buf(),
            inode,
            offset,
        });
        let mut chunk = Vec::new();
        if file.seek(SeekFrom::Start(followed.offset)).is_err()
            || (&mut file)
                .take(READ_BUDGET)
                .read_to_end(&mut chunk)
                .is_err()
        {
            return TranscriptRead {
                reset,
                entries: Vec::new(),
            };
        }
        let complete = chunk
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |at| at + 1);
        let mut entries = Vec::new();
        for line in chunk[..complete].split_inclusive(|byte| *byte == b'\n') {
            if std::mem::take(&mut self.skipping) {
                continue;
            }
            entries.extend(message_entries(line));
        }
        followed.offset += complete as u64;
        if complete == 0 && chunk.len() as u64 == READ_BUDGET {
            followed.offset += READ_BUDGET;
            self.skipping = true;
        }
        TranscriptRead { reset, entries }
    }
}

fn open_within(profile: &Path, path: &Path) -> Option<File> {
    let root = std::fs::canonicalize(profile).ok()?.join(DIR);
    let file = File::options()
        .read(true)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
        .open(path)
        .ok()?;
    let opened = std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).ok()?;
    let meta = file.metadata().ok()?;
    (meta.is_file() && meta.nlink() == 1 && opened.starts_with(&root)).then_some(file)
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
