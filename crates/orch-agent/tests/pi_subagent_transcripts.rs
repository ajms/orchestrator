use std::io::Write;
use std::path::{Path, PathBuf};

use orch_agent::{AgentAdapter, Pi, SubagentTranscripts, TranscriptRead, TranscriptReader};
use orch_core::{SubagentId, TranscriptEntry};
use serde_json::{Value, json};

const CALL: &str = "call_subagent";
const CONVERSATION: &str = "4c7f99b3-90ed-4ada-87e9-793165b1a364";

struct Profile {
    state: tempfile::TempDir,
    transcripts: Box<dyn SubagentTranscripts>,
}

impl Profile {
    fn new() -> Self {
        let state = tempfile::tempdir().unwrap();
        let transcripts = Self::transcripts(state.path());
        Self { state, transcripts }
    }

    fn transcripts(state: &Path) -> Box<dyn SubagentTranscripts> {
        let state = state.to_string_lossy().into_owned();
        Pi::default()
            .subagent_transcripts(&|key| match key {
                "HOME" => Some("/home/dev".into()),
                "XDG_STATE_HOME" => Some(state.clone()),
                _ => None,
            })
            .unwrap()
    }

    fn dir(&self) -> PathBuf {
        self.state.path().join("orchestrator/agent-profiles/pi")
    }

    fn file(&self, conversation: &str, call: &str) -> PathBuf {
        self.dir()
            .join("subagents")
            .join(conversation)
            .join(format!("{call}.jsonl"))
    }

    fn follow(&mut self, payload: &Value) {
        self.transcripts.follow(&payload.to_string());
    }

    fn started(&mut self) -> PathBuf {
        self.follow(&hook("tool_call_subagent"));
        self.transcripts
            .locate(&SubagentId(CALL.into()))
            .expect("a located transcript")
    }

    fn reader(&self) -> Box<dyn TranscriptReader> {
        self.transcripts.reader()
    }
}

fn fixture(kind: &str, name: &str) -> PathBuf {
    format!(
        "{}/tests/fixtures/pi/{kind}/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
    .into()
}

fn hook(name: &str) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(fixture("hooks", &format!("{name}.json"))).unwrap(),
    )
    .unwrap()
}

fn call(conversation: &str, id: &str) -> Value {
    json!({"event": "tool_call", "toolName": "subagent", "toolCallId": id, "sessionId": conversation, "agent": "scout", "taskName": "look"})
}

fn said(text: &str) -> String {
    json!({"role": "assistant", "content": [{"type": "text", "text": text}]}).to_string()
}

fn text(text: &str) -> TranscriptEntry {
    TranscriptEntry::Text { text: text.into() }
}

fn write(path: &Path, lines: &[String]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        lines
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>(),
    )
    .unwrap();
}

fn append(path: &Path, content: &str) {
    let mut file = std::fs::File::options().append(true).open(path).unwrap();
    file.write_all(content.as_bytes()).unwrap();
}

fn replace(path: &Path, lines: &[String]) {
    let staged = path.with_extension("jsonl.new");
    write(&staged, lines);
    std::fs::rename(staged, path).unwrap();
}

#[test]
fn a_subagent_calls_transcript_sits_in_the_profile_under_its_conversation() {
    let mut profile = Profile::new();
    assert_eq!(profile.started(), profile.file(CONVERSATION, CALL));
}

#[test]
fn a_subagent_keeps_its_transcript_when_the_conversation_changes() {
    let mut profile = Profile::new();
    let before = profile.started();
    profile.follow(&json!({"event": "session_start", "reason": "new", "sessionId": "conv-2"}));
    profile.follow(&hook("tool_execution_end_subagent"));
    assert_eq!(
        profile.transcripts.locate(&SubagentId(CALL.into())),
        Some(before)
    );
}

#[test]
fn only_subagent_calls_have_transcripts() {
    let mut profile = Profile::new();
    profile.follow(&json!({"event": "tool_call", "toolName": "bash", "toolCallId": "call_1", "sessionId": "conv-1"}));
    profile.follow(&json!({"event": "tool_call", "toolName": "subagent", "toolCallId": "call_2"}));
    for id in ["call_1", "call_2", "nobody"] {
        assert_eq!(
            profile.transcripts.locate(&SubagentId(id.into())),
            None,
            "{id}"
        );
    }
}

#[test]
fn ids_that_arent_plain_file_names_have_no_transcript() {
    let mut profile = Profile::new();
    for (conversation, id) in [
        ("..", "call_1"),
        ("conv/1", "call_2"),
        ("conv-1", "../../escape"),
        ("conv-1", "parent/1"),
        ("conv-1", ".."),
        ("", "call_3"),
        ("conv-1", ""),
    ] {
        profile.follow(&call(conversation, id));
        assert_eq!(
            profile.transcripts.locate(&SubagentId(id.into())),
            None,
            "{conversation} {id}"
        );
    }
}

#[test]
fn without_home_pi_keeps_no_subagent_transcripts() {
    assert!(Pi::default().subagent_transcripts(&|_| None).is_none());
}

#[test]
fn the_childs_messages_map_to_entries_and_thinking_is_dropped() {
    let mut profile = Profile::new();
    let path = profile.started();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::copy(fixture("transcripts", "subagent.jsonl"), &path).unwrap();

    let read = profile.reader().read(&path);

    assert!(read.reset);
    assert_eq!(
        read.entries,
        [
            TranscriptEntry::Prompt {
                text: "Task: Find where login redirects".into()
            },
            text("I'll search the sources."),
            TranscriptEntry::ToolCall {
                id: "call_grep".into(),
                tool: "grep".into(),
                argument: Some("redirect".into()),
            },
            TranscriptEntry::ToolResult {
                id: "call_grep".into(),
                text: "src/login.rs:42: redirect(next)".into(),
                error: false,
            },
            TranscriptEntry::ToolCall {
                id: "call_read".into(),
                tool: "read".into(),
                argument: Some("src/login.rs".into()),
            },
            TranscriptEntry::ToolResult {
                id: "call_read".into(),
                text: "ENOENT: no such file".into(),
                error: true,
            },
            text("Found the redirect in src/login.rs:42."),
        ]
    );
}

#[test]
fn a_tool_call_names_claudes_key_argument_before_pis_path() {
    let mut profile = Profile::new();
    let path = profile.started();
    let tool = |name: &str, arguments: Value| {
        json!({"role": "assistant", "content": [{"type": "toolCall", "id": name, "name": name, "arguments": arguments}]})
            .to_string()
    };
    write(
        &path,
        &[
            tool("bash", json!({"command": "ls", "timeout": 5})),
            tool("find", json!({"path": "src", "pattern": "*.rs"})),
            tool("edit", json!({"path": "src/a.rs", "oldText": "a"})),
            tool("todo", json!({"items": ["x"]})),
        ],
    );
    let arguments: Vec<Option<String>> = profile
        .reader()
        .read(&path)
        .entries
        .into_iter()
        .map(|entry| match entry {
            TranscriptEntry::ToolCall { argument, .. } => argument,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        arguments,
        [
            Some("ls".into()),
            Some("*.rs".into()),
            Some("src/a.rs".into()),
            None
        ]
    );
}

#[test]
fn a_prompt_may_be_plain_text_and_other_roles_are_dropped() {
    let mut profile = Profile::new();
    let path = profile.started();
    write(
        &path,
        &[
            json!({"role": "user", "content": "Find it"}).to_string(),
            json!({"role": "system", "content": "Be brief"}).to_string(),
            json!({"role": "bashExecution", "command": "ls"}).to_string(),
        ],
    );
    assert_eq!(
        profile.reader().read(&path).entries,
        [TranscriptEntry::Prompt {
            text: "Find it".into()
        }]
    );
}

#[test]
fn each_read_returns_only_the_messages_appended_since() {
    let mut profile = Profile::new();
    let path = profile.started();
    write(&path, &[said("Searching.")]);
    let mut reader = profile.reader();
    reader.read(&path);

    append(&path, &format!("{}\n", said("Found it.")));

    assert_eq!(
        reader.read(&path),
        TranscriptRead {
            reset: false,
            entries: vec![text("Found it.")],
        }
    );
}

#[test]
fn a_message_still_being_written_waits_for_its_newline() {
    let mut profile = Profile::new();
    let path = profile.started();
    write(&path, &[]);
    let mut reader = profile.reader();
    let line = said("Found it.");
    let (head, tail) = line.split_at(10);
    append(&path, head);
    assert_eq!(reader.read(&path).entries, []);
    append(&path, &format!("{tail}\n"));
    assert_eq!(reader.read(&path).entries, [text("Found it.")]);
}

#[test]
fn a_retry_that_replaces_the_file_reads_as_a_reset() {
    let mut profile = Profile::new();
    let path = profile.started();
    write(&path, &[said("Overloaded.")]);
    let mut reader = profile.reader();
    reader.read(&path);

    replace(
        &path,
        &[said("Starting over, with a longer first message.")],
    );

    assert_eq!(
        reader.read(&path),
        TranscriptRead {
            reset: true,
            entries: vec![text("Starting over, with a longer first message.")],
        }
    );
}

#[test]
fn a_shrunk_file_reads_as_a_reset() {
    let mut profile = Profile::new();
    let path = profile.started();
    write(&path, &[said("One."), said("Two.")]);
    let mut reader = profile.reader();
    reader.read(&path);

    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(0)
        .unwrap();
    append(&path, &format!("{}\n", said("Again.")));

    let read = reader.read(&path);
    assert!(read.reset);
    assert_eq!(read.entries, [text("Again.")]);
}

#[test]
fn garbage_lines_are_skipped() {
    let mut profile = Profile::new();
    let path = profile.started();
    write(
        &path,
        &[
            "not json".into(),
            "[1,2]".into(),
            json!({"role": "assistant", "content": "not blocks"}).to_string(),
            json!({"role": "assistant", "content": [{"type": "toolCall"}]}).to_string(),
            said("Fine."),
        ],
    );
    assert_eq!(profile.reader().read(&path).entries, [text("Fine.")]);
}

#[test]
fn an_overlong_line_is_skipped_and_the_rest_still_read() {
    let mut profile = Profile::new();
    let path = profile.started();
    let huge = said(&"x".repeat(20 * 1024 * 1024));
    write(&path, &[huge, said("After.")]);
    let mut reader = profile.reader();
    let entries: Vec<TranscriptEntry> = (0..10).flat_map(|_| reader.read(&path).entries).collect();
    assert_eq!(entries, [text("After.")]);
}

#[test]
fn a_transcript_linked_out_of_the_profile_is_not_read() {
    let mut profile = Profile::new();
    let path = profile.started();
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret.jsonl");
    write(&secret, &[said("secret")]);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&secret, &path).unwrap();

    assert_eq!(profile.reader().read(&path).entries, []);
}

#[test]
fn a_conversation_dir_linked_out_of_the_profile_is_not_read() {
    let mut profile = Profile::new();
    let path = profile.started();
    let outside = tempfile::tempdir().unwrap();
    write(
        &outside.path().join(format!("{CALL}.jsonl")),
        &[said("secret")],
    );
    std::fs::create_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(outside.path(), path.parent().unwrap()).unwrap();

    assert_eq!(profile.reader().read(&path).entries, []);
}

#[test]
fn a_subagents_dir_linked_out_of_the_profile_is_not_read() {
    let mut profile = Profile::new();
    let path = profile.started();
    let outside = tempfile::tempdir().unwrap();
    write(
        &outside
            .path()
            .join(CONVERSATION)
            .join(format!("{CALL}.jsonl")),
        &[said("secret")],
    );
    std::fs::create_dir_all(profile.dir()).unwrap();
    std::os::unix::fs::symlink(outside.path(), profile.dir().join("subagents")).unwrap();

    assert_eq!(profile.reader().read(&path).entries, []);
}

#[test]
fn a_path_outside_the_profiles_transcripts_is_not_read() {
    let profile = Profile::new();
    let outside = tempfile::tempdir().unwrap();
    let path = outside.path().join("transcript.jsonl");
    write(&path, &[said("secret")]);
    assert_eq!(profile.reader().read(&path).entries, []);
}

#[test]
fn a_fifo_in_place_of_the_transcript_is_not_waited_on() {
    let mut profile = Profile::new();
    let path = profile.started();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let made = std::process::Command::new("mkfifo")
        .arg(&path)
        .status()
        .unwrap();
    assert!(made.success());
    assert_eq!(profile.reader().read(&path).entries, []);
}

#[test]
fn a_finished_subagents_transcript_stays_readable() {
    let mut profile = Profile::new();
    let path = profile.started();
    write(&path, &[said("Done.")]);
    profile.follow(&hook("tool_execution_end_subagent"));
    let located = profile
        .transcripts
        .locate(&SubagentId(CALL.into()))
        .unwrap();
    assert_eq!(profile.reader().read(&located).entries, [text("Done.")]);
}
