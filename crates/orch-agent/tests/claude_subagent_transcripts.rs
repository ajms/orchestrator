use std::io::Write;
use std::path::{Path, PathBuf};

use orch_agent::{AgentAdapter, ClaudeCode, SubagentTranscripts, TranscriptReader};
use orch_core::{SubagentId, TranscriptEntry};
use serde_json::{Value, json};

const SUBAGENT: &str = "a7f3c9e1b2d4";
const SESSION_TRANSCRIPT: &str = "/home/dev/.claude/projects/-home-dev-shop--orchestrator-worktrees-fix-login/5f0c6a52-6a3e-4d3b-9d7e-0b1f8c1e2a90.jsonl";

fn fixture_path(kind: &str, name: &str) -> PathBuf {
    format!(
        "{}/tests/fixtures/{kind}/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
    .into()
}

fn hook(name: &str) -> Value {
    let path = fixture_path("hooks", &format!("{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn claude_transcripts() -> Box<dyn SubagentTranscripts> {
    ClaudeCode::default().subagent_transcripts().unwrap()
}

fn follow(transcripts: &mut Box<dyn SubagentTranscripts>, payload: &Value) {
    transcripts.follow(&payload.to_string());
}

fn locate(transcripts: &dyn SubagentTranscripts) -> Option<PathBuf> {
    transcripts.locate(&SubagentId(SUBAGENT.into()))
}

fn reader() -> Box<dyn TranscriptReader> {
    claude_transcripts().reader()
}

fn prompt(text: &str) -> String {
    json!({"type": "user", "message": {"role": "user", "content": text}}).to_string()
}

fn said(text: &str) -> String {
    json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": text}]}})
        .to_string()
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

fn text(text: &str) -> TranscriptEntry {
    TranscriptEntry::Text { text: text.into() }
}

#[test]
fn a_running_subagents_transcript_sits_in_the_sessions_transcript_folder() {
    let mut transcripts = claude_transcripts();
    follow(&mut transcripts, &hook("subagent_start"));
    let folder = SESSION_TRANSCRIPT.strip_suffix(".jsonl").unwrap();
    assert_eq!(
        locate(transcripts.as_ref()),
        Some(PathBuf::from(format!(
            "{folder}/subagents/agent-{SUBAGENT}.jsonl"
        )))
    );
}

#[test]
fn subagent_stop_names_the_transcript() {
    let mut transcripts = claude_transcripts();
    follow(&mut transcripts, &hook("subagent_start"));
    follow(&mut transcripts, &hook("subagent_stop"));
    assert_eq!(
        locate(transcripts.as_ref()),
        Some(PathBuf::from(format!(
            "/home/dev/.claude/projects/x/5f0c6a52-6a3e-4d3b-9d7e-0b1f8c1e2a90/subagents/agent-{SUBAGENT}.jsonl"
        )))
    );
}

#[test]
fn a_subagent_keeps_its_transcript_when_the_conversation_changes() {
    let mut transcripts = claude_transcripts();
    follow(&mut transcripts, &hook("subagent_start"));
    let before = locate(transcripts.as_ref());
    let mut cleared = hook("session_start_clear");
    cleared["transcript_path"] = "/home/dev/.claude/projects/y/other.jsonl".into();
    follow(&mut transcripts, &cleared);
    assert_eq!(locate(transcripts.as_ref()), before);
}

#[test]
fn an_unknown_subagent_has_no_transcript() {
    let mut transcripts = claude_transcripts();
    follow(&mut transcripts, &hook("stop"));
    assert_eq!(locate(transcripts.as_ref()), None);
}

#[test]
fn every_entry_kind_is_mapped_and_thinking_meta_lines_and_interruptions_are_dropped() {
    let read = reader().read(&fixture_path("transcripts", "subagent.jsonl"));
    assert!(read.reset);
    assert_eq!(
        read.entries,
        [
            TranscriptEntry::Prompt {
                text: "Find where the login redirect happens".into()
            },
            text("I'll search the sources."),
            TranscriptEntry::ToolCall {
                id: "toolu_01".into(),
                tool: "Grep".into(),
                argument: Some("redirect".into()),
            },
            TranscriptEntry::ToolResult {
                id: "toolu_01".into(),
                text: "src/login.rs:42: redirect(next)\nsrc/login.rs:57: redirect(home)".into(),
                error: false,
            },
            TranscriptEntry::ToolCall {
                id: "toolu_02".into(),
                tool: "Bash".into(),
                argument: Some("cargo test login".into()),
            },
            TranscriptEntry::ToolResult {
                id: "toolu_02".into(),
                text: "error: no test named login".into(),
                error: true,
            },
            text("Found the redirect in src/login.rs:42."),
        ]
    );
}

#[test]
fn a_tool_call_names_its_key_argument() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let call = |name: &str, input: Value| {
        json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": name, "name": name, "input": input}]}})
            .to_string()
    };
    append(
        file.path(),
        &[
            call("Read", json!({"file_path": "src/login.rs", "limit": 20})),
            call(
                "WebFetch",
                json!({"url": "https://example.com", "prompt": "summarise"}),
            ),
            call("TodoWrite", json!({"todos": []})),
        ],
    );
    let arguments: Vec<Option<String>> = reader()
        .read(file.path())
        .entries
        .into_iter()
        .map(|entry| match entry {
            TranscriptEntry::ToolCall { argument, .. } => argument,
            other => panic!("not a tool call: {other:?}"),
        })
        .collect();
    assert_eq!(
        arguments,
        [
            Some("src/login.rs".into()),
            Some("https://example.com".into()),
            None
        ]
    );
}

#[test]
fn appended_lines_are_read_once_and_a_half_written_line_waits() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut reader = reader();
    append(file.path(), &[prompt("Look around"), said("first")]);
    let read = reader.read(file.path());
    assert!(read.reset);
    assert_eq!(read.entries.len(), 2);

    let read = reader.read(file.path());
    assert!(!read.reset);
    assert_eq!(read.entries, []);

    let half = said("second");
    let mut writer = std::fs::File::options()
        .append(true)
        .open(file.path())
        .unwrap();
    write!(writer, "{}", &half[..20]).unwrap();
    assert_eq!(reader.read(file.path()).entries, []);

    writeln!(writer, "{}", &half[20..]).unwrap();
    let read = reader.read(file.path());
    assert!(!read.reset);
    assert_eq!(read.entries, [text("second")]);
}

#[test]
fn a_shrunk_transcript_is_read_again_from_the_start() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let mut reader = reader();
    append(
        file.path(),
        &[prompt("Look around"), said("a long first reply")],
    );
    reader.read(file.path());

    std::fs::write(file.path(), format!("{}\n", said("short"))).unwrap();
    let read = reader.read(file.path());
    assert!(read.reset);
    assert_eq!(read.entries, [text("short")]);
}

#[test]
fn another_transcript_is_read_from_the_start() {
    let first = tempfile::NamedTempFile::new().unwrap();
    let second = tempfile::NamedTempFile::new().unwrap();
    let mut reader = reader();
    append(first.path(), &[said("first")]);
    append(second.path(), &[said("second"), said("third")]);
    reader.read(first.path());

    let read = reader.read(second.path());
    assert!(read.reset);
    assert_eq!(read.entries, [text("second"), text("third")]);
}

#[test]
fn a_missing_transcript_reads_as_nothing_yet() {
    let read = reader().read(Path::new("/nonexistent/agent-x.jsonl"));
    assert_eq!(read.entries, []);
}
