use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use orch_core::{SubagentId, TranscriptEntry};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::lines::FollowedLines;
use crate::{SubagentTranscripts, TranscriptRead, TranscriptReader};

const SYSTEM_SENDER: &str = "system";
const TIMESTAMPS: [&str; 2] = ["Created At: ", "Completed At: "];
const KEY_ARGUMENTS: [&str; 10] = [
    "CommandLine",
    "TargetFile",
    "AbsolutePath",
    "Prompt",
    "Message",
    "Query",
    "Url",
    "DirectoryPath",
    "SearchPath",
    "Question",
];

#[derive(Debug, Default)]
pub(super) struct AntigravitySubagentTranscripts {
    paths: HashMap<String, PathBuf>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HookPaths {
    conversation_id: String,
    transcript_path: PathBuf,
}

impl SubagentTranscripts for AntigravitySubagentTranscripts {
    fn follow(&mut self, payload: &str) {
        if let Ok(hook) = serde_json::from_str::<HookPaths>(payload) {
            self.paths
                .insert(hook.conversation_id, hook.transcript_path);
        }
    }

    fn locate(&self, subagent: &SubagentId) -> Option<PathBuf> {
        self.paths.get(subagent.as_str()).cloned()
    }

    fn reader(&self) -> Box<dyn TranscriptReader> {
        Box::new(StepReader::default())
    }
}

#[derive(Debug, Default)]
struct StepReader {
    lines: FollowedLines,
    calls: VecDeque<String>,
}

impl TranscriptReader for StepReader {
    fn read(&mut self, path: &Path) -> TranscriptRead {
        let Some(read) = self.lines.read(path) else {
            return TranscriptRead::default();
        };
        if read.reset {
            self.calls.clear();
        }
        let entries = read
            .lines
            .iter()
            .filter_map(|line| serde_json::from_slice::<Step>(line).ok())
            .flat_map(|step| self.entries(step))
            .collect();
        TranscriptRead {
            reset: read.reset,
            entries,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Step {
    step_index: u64,
    #[serde(rename = "type")]
    kind: String,
    status: String,
    content: String,
    tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ToolCall {
    name: String,
    args: Map<String, Value>,
}

impl StepReader {
    fn entries(&mut self, step: Step) -> Vec<TranscriptEntry> {
        match step.kind.as_str() {
            "PLANNER_RESPONSE" => {
                let text = Some(step.content.trim())
                    .filter(|text| !text.is_empty())
                    .map(|text| TranscriptEntry::Text { text: text.into() });
                let calls = step.tool_calls.into_iter().enumerate().map(|(at, call)| {
                    let id = format!("{}.{at}", step.step_index);
                    self.calls.push_back(id.clone());
                    TranscriptEntry::ToolCall {
                        id,
                        argument: key_argument(&call.args),
                        tool: call.name,
                    }
                });
                text.into_iter().chain(calls).collect()
            }
            "GENERIC" => self
                .calls
                .pop_front()
                .map(|id| TranscriptEntry::ToolResult {
                    id,
                    text: without_timestamps(&step.content),
                    error: step.status == "ERROR",
                })
                .into_iter()
                .collect(),
            _ => prompt(&step.kind, &step.content)
                .map(|text| TranscriptEntry::Prompt { text })
                .into_iter()
                .collect(),
        }
    }
}

pub(super) fn first_prompt(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str::<Step>(&line).ok())
        .find_map(|step| prompt(&step.kind, &step.content))
}

fn prompt(kind: &str, content: &str) -> Option<String> {
    match kind {
        "USER_INPUT" => Some(user_request(content)),
        "SYSTEM_MESSAGE" => sent_message(content),
        _ => None,
    }
}

fn user_request(content: &str) -> String {
    let request = content
        .split_once("<USER_REQUEST>")
        .and_then(|(_, rest)| rest.split_once("</USER_REQUEST>"))
        .map_or(content, |(request, _)| request);
    request.trim().into()
}

fn sent_message(content: &str) -> Option<String> {
    let (_, message) = content.split_once("[Message] ")?;
    let (header, text) = message.split_once(" content=")?;
    let sender = header
        .split_whitespace()
        .find_map(|field| field.strip_prefix("sender="))?;
    if sender == SYSTEM_SENDER {
        return None;
    }
    let text = text
        .split_once("</SYSTEM_MESSAGE>")
        .map_or(text, |(text, _)| text);
    Some(text.trim().into())
}

fn without_timestamps(content: &str) -> String {
    content
        .lines()
        .filter(|line| !TIMESTAMPS.iter().any(|stamp| line.starts_with(stamp)))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .into()
}

fn key_argument(args: &Map<String, Value>) -> Option<String> {
    KEY_ARGUMENTS
        .iter()
        .find_map(|key| args.get(*key)?.as_str())
        .map(String::from)
}
