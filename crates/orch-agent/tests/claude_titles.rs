use std::io::Write;
use std::path::Path;

use orch_agent::{AgentAdapter, ClaudeCode, TitleWatch};
use orch_core::AgentEvent;
use serde_json::Value;

fn fixture_path(kind: &str, name: &str) -> String {
    format!(
        "{}/tests/fixtures/{kind}/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn fixture(kind: &str, name: &str) -> Value {
    let path = fixture_path(kind, name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path}: {err}"));
    serde_json::from_str(&text).unwrap()
}

fn hook(name: &str) -> Value {
    fixture("hooks", &format!("{name}.json"))
}

fn reading(mut payload: Value, transcript: &Path) -> Value {
    payload["transcript_path"] = transcript.to_string_lossy().into();
    payload
}

fn hook_reading(name: &str, transcript: &Path) -> Value {
    reading(hook(name), transcript)
}

fn transcript(name: &str) -> std::path::PathBuf {
    fixture_path("transcripts", &format!("{name}.jsonl")).into()
}

fn claude_watch() -> Box<dyn TitleWatch> {
    ClaudeCode::default().title_watch().unwrap()
}

fn follow(watch: &mut Box<dyn TitleWatch>, payload: &Value) -> Vec<String> {
    watch.follow(&payload.to_string());
    titles(watch)
}

fn titles(watch: &mut Box<dyn TitleWatch>) -> Vec<String> {
    watch
        .poll()
        .into_iter()
        .filter_map(|event| match event {
            AgentEvent::TitleChanged { title } => Some(title),
            _ => None,
        })
        .collect()
}

fn rename(title: &str) -> String {
    format!(r#"{{"type":"custom-title","customTitle":"{title}","sessionId":"5f0c6a52"}}"#)
}

#[test]
fn hooks_map_without_reading_the_title() {
    let claude = ClaudeCode::default();
    let payload = hook_reading("user_prompt_submit_titled", &transcript("renamed"));
    let events = claude.map_hook(&payload.to_string()).unwrap();
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, AgentEvent::TitleChanged { .. })),
        "{events:?}"
    );
}

#[test]
fn the_latest_rename_in_the_transcript_is_the_title() {
    let mut watch = claude_watch();
    assert_eq!(
        follow(&mut watch, &hook_reading("stop", &transcript("renamed"))),
        ["login redirect"]
    );
}

#[test]
fn the_statusline_names_the_transcript_too() {
    let mut watch = claude_watch();
    let tap = reading(
        fixture("statusline", "subscriber.json"),
        &transcript("renamed"),
    );
    assert_eq!(follow(&mut watch, &tap), ["login redirect"]);
}

#[test]
fn generated_titles_are_not_session_titles() {
    for hook_name in ["stop", "session_start_startup", "user_prompt_submit"] {
        let mut watch = claude_watch();
        let payload = hook_reading(hook_name, &transcript("unnamed"));
        assert_eq!(
            follow(&mut watch, &payload),
            Vec::<String>::new(),
            "{hook_name}"
        );
    }
}

#[test]
fn a_rename_is_reported_once_without_another_hook() {
    let mut watch = claude_watch();
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "{}", rename("first")).unwrap();
    assert_eq!(
        follow(&mut watch, &hook_reading("stop", file.path())),
        ["first"]
    );
    assert_eq!(titles(&mut watch), Vec::<String>::new());

    writeln!(file, "{}", rename("second")).unwrap();
    assert_eq!(titles(&mut watch), ["second"]);

    writeln!(file, "{}", rename("")).unwrap();
    assert_eq!(titles(&mut watch), [""]);
}

#[test]
fn a_rewritten_transcript_is_read_again() {
    let mut watch = claude_watch();
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "{}", rename("before compaction, a long title")).unwrap();
    assert_eq!(
        follow(&mut watch, &hook_reading("stop", file.path())),
        ["before compaction, a long title"]
    );

    std::fs::write(file.path(), format!("{}\n", rename("after"))).unwrap();
    assert_eq!(titles(&mut watch), ["after"]);
}

#[test]
fn a_rename_still_being_written_waits_for_its_line_to_finish() {
    let mut watch = claude_watch();
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "{}", rename("done")).unwrap();
    let half = rename("half written");
    write!(file, "{}", &half[..20]).unwrap();
    assert_eq!(
        follow(&mut watch, &hook_reading("stop", file.path())),
        ["done"]
    );

    writeln!(file, "{}", &half[20..]).unwrap();
    assert_eq!(titles(&mut watch), ["half written"]);
}

#[test]
fn the_hooks_title_fills_in_while_the_transcript_has_none() {
    let mut watch = claude_watch();
    assert_eq!(
        follow(&mut watch, &hook("user_prompt_submit_titled")),
        ["login redirect"]
    );

    let mut watch = claude_watch();
    assert_eq!(
        follow(
            &mut watch,
            &hook_reading("session_start_resume_titled", &transcript("unnamed"))
        ),
        ["login redirect"]
    );
}

#[test]
fn the_transcript_wins_over_the_hooks_title() {
    let mut watch = claude_watch();
    let mut payload = hook_reading("user_prompt_submit_titled", &transcript("renamed"));
    payload["session_title"] = "something else".into();
    assert_eq!(follow(&mut watch, &payload), ["login redirect"]);
}

#[test]
fn a_rename_in_the_transcript_replaces_the_hooks_title() {
    let mut watch = claude_watch();
    let mut file = tempfile::NamedTempFile::new().unwrap();
    assert_eq!(
        follow(
            &mut watch,
            &hook_reading("user_prompt_submit_titled", file.path())
        ),
        ["login redirect"]
    );

    writeln!(file, "{}", rename("logout page")).unwrap();
    assert_eq!(titles(&mut watch), ["logout page"]);
    assert_eq!(
        follow(
            &mut watch,
            &hook_reading("user_prompt_submit_titled", file.path())
        ),
        Vec::<String>::new()
    );
}

#[test]
fn an_empty_title_from_the_hook_clears_it() {
    let mut watch = claude_watch();
    let mut payload = hook("user_prompt_submit_titled");
    payload["session_title"] = "".into();
    assert_eq!(follow(&mut watch, &payload), [""]);
}

#[test]
fn only_prompt_and_start_hooks_carry_the_title() {
    let mut watch = claude_watch();
    let mut payload = hook("pre_tool_use_bash");
    payload["session_title"] = "login redirect".into();
    assert_eq!(follow(&mut watch, &payload), Vec::<String>::new());
}

#[test]
fn a_new_conversation_keeps_the_title_until_renamed() {
    let mut watch = claude_watch();
    let mut first = tempfile::NamedTempFile::new().unwrap();
    follow(
        &mut watch,
        &hook_reading("user_prompt_submit_titled", first.path()),
    );
    writeln!(first, "{}", rename("logout page")).unwrap();
    assert_eq!(titles(&mut watch), ["logout page"]);

    let cleared = hook_reading("session_start_clear", &transcript("unnamed"));
    assert_eq!(follow(&mut watch, &cleared), Vec::<String>::new());
}
