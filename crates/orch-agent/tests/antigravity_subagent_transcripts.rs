use std::io::Write;
use std::path::{Path, PathBuf};

use orch_agent::{AgentAdapter, Antigravity, SubagentTranscripts, TranscriptReader};
use orch_core::{SubagentId, TranscriptEntry};
use serde_json::json;

const CHILD: &str = "8d2f61b7-4a09-4c3e-9b15-e07a3c5d9f28";

fn fixture_path(name: &str) -> PathBuf {
    format!(
        "{}/tests/fixtures/antigravity/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
    .into()
}

fn transcripts() -> Box<dyn SubagentTranscripts> {
    Antigravity::default().subagent_transcripts().unwrap()
}

fn reader() -> Box<dyn TranscriptReader> {
    transcripts().reader()
}

fn follow(transcripts: &mut Box<dyn SubagentTranscripts>, name: &str) {
    let payload = std::fs::read_to_string(fixture_path(&format!("hooks/{name}.json"))).unwrap();
    transcripts.follow(&payload);
}

fn locate(transcripts: &dyn SubagentTranscripts, id: &str) -> Option<PathBuf> {
    transcripts.locate(&SubagentId(id.into()))
}

fn append(path: &Path, lines: &[String]) {
    let mut file = std::fs::File::options()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    for line in lines {
        writeln!(file, "{line}").unwrap();
    }
}

fn planned(index: usize, content: &str, calls: serde_json::Value) -> String {
    json!({"step_index": index, "source": "MODEL", "type": "PLANNER_RESPONSE", "status": "DONE", "content": content, "tool_calls": calls}).to_string()
}

fn result(index: usize, content: &str) -> String {
    json!({"step_index": index, "source": "MODEL", "type": "GENERIC", "status": "DONE", "content": content}).to_string()
}

fn text(text: &str) -> TranscriptEntry {
    TranscriptEntry::Text { text: text.into() }
}

#[test]
fn a_subagents_transcript_is_the_one_its_hooks_name() {
    let mut transcripts = transcripts();
    follow(&mut transcripts, "subagent_pre_invocation");
    assert_eq!(
        locate(transcripts.as_ref(), CHILD),
        Some(PathBuf::from(format!(
            "/home/dev/.gemini/antigravity-cli/brain/{CHILD}/.system_generated/logs/transcript_full.jsonl"
        )))
    );
    assert_eq!(
        locate(transcripts.as_ref(), "5b7e0c94-1f3a-4d62-8a08-c4e9f2b17d35"),
        None
    );
}

#[test]
fn every_step_kind_is_mapped_and_thinking_and_system_notices_are_dropped() {
    let read = reader().read(&fixture_path("transcripts/subagent_full.jsonl"));
    assert!(read.reset);
    assert_eq!(
        read.entries,
        [
            TranscriptEntry::Prompt {
                text: "Run the login tests and report the failures.\nUse cargo test.".into()
            },
            TranscriptEntry::ToolCall {
                id: "1.0".into(),
                tool: "view_file".into(),
                argument: Some(
                    "/home/dev/shop/.orchestrator/worktrees/fix-login/src/login.rs".into()
                ),
            },
            TranscriptEntry::ToolResult {
                id: "1.0".into(),
                text: "File Path: `file:///home/dev/shop/.orchestrator/worktrees/fix-login/src/login.rs`\nTotal Lines: 2".into(),
                error: false,
            },
            text("Running the login tests now."),
            TranscriptEntry::ToolCall {
                id: "3.0".into(),
                tool: "run_command".into(),
                argument: Some("cargo test login".into()),
            },
            TranscriptEntry::ToolCall {
                id: "3.1".into(),
                tool: "list_dir".into(),
                argument: None,
            },
            TranscriptEntry::ToolResult {
                id: "3.0".into(),
                text: "Encountered error in step execution: tool call denied by pre-tool hook:"
                    .into(),
                error: true,
            },
            TranscriptEntry::ToolResult {
                id: "3.1".into(),
                text: "tests/login.rs".into(),
                error: false,
            },
            TranscriptEntry::Prompt {
                text: "Only report the first failure.".into()
            },
            text("The first failure is login_redirects_home."),
            TranscriptEntry::ToolCall {
                id: "8.0".into(),
                tool: "send_message".into(),
                argument: Some("login_redirects_home fails.".into()),
            },
            TranscriptEntry::ToolResult {
                id: "8.0".into(),
                text: "Message sent to \"3c1e9a40-7d52-4b8e-a6f1-2d9b0c4e7a13\".".into(),
                error: false,
            },
        ]
    );
}

#[test]
fn a_result_written_after_its_call_still_pairs_with_it() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut reader = reader();
    let call = json!([{"name": "grep_search", "args": {"Query": "redirect", "SearchPath": "src"}}]);
    append(file.path(), &[planned(4, "", call)]);
    assert_eq!(
        reader.read(file.path()).entries,
        [TranscriptEntry::ToolCall {
            id: "4.0".into(),
            tool: "grep_search".into(),
            argument: Some("redirect".into()),
        }]
    );

    append(file.path(), &[result(5, "src/login.rs:42")]);
    let read = reader.read(file.path());
    assert!(!read.reset);
    assert_eq!(
        read.entries,
        [TranscriptEntry::ToolResult {
            id: "4.0".into(),
            text: "src/login.rs:42".into(),
            error: false,
        }]
    );
}
