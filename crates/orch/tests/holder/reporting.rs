use std::time::{Duration, Instant};

use orch_agent::ClaudeCode;
use orch_holder::HolderEvent;

use crate::common::*;

const STATUS: &str =
    r#"{"model":{"display_name":"Opus"},"context_window":{"used_percentage":41.6}}"#;
const PRE_TOOL_USE: &str =
    r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"ls"}}"#;

#[test]
fn hook_exits_quietly_when_no_holder_runs() {
    let sandbox = Sandbox::new();
    let started = Instant::now();

    let output = run_with_stdin(
        sandbox.orch().args(["hook", "--session", "gone"]),
        r#"{"hook_event_name":"Stop"}"#,
    );

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn guard_hook_without_a_holder_asks_the_user() {
    let sandbox = Sandbox::new();

    let output = run_with_stdin(
        sandbox.orch().args(["hook", "--session", "gone"]),
        PRE_TOOL_USE,
    );

    assert!(output.status.success());
    let answer: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(answer["hookSpecificOutput"]["permissionDecision"], "ask");
}

#[test]
fn hook_survives_garbage_input() {
    let sandbox = Sandbox::new();

    let output = run_with_stdin(
        sandbox.orch().args(["hook", "--session", "gone"]),
        "not json",
    );

    assert!(output.status.success());
}

#[test]
fn tap_prints_a_minimal_line_when_the_user_has_no_statusline() {
    let sandbox = Sandbox::new();

    let output = run_with_stdin(sandbox.orch().args(["tap", "--session", "gone"]), STATUS);

    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "Opus · 42%\n");
}

#[test]
fn tap_output_is_identical_to_the_users_own_statusline() {
    let sandbox = Sandbox::new();
    sandbox.write_claude_settings(
        r#"{"statusLine":{"type":"command","command":"printf '\\033[1mmine\\033[0m '; cat; printf '\\nsecond line'"}}"#,
    );

    let output = run_with_stdin(sandbox.orch().args(["tap", "--session", "gone"]), STATUS);

    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("\x1b[1mmine\x1b[0m {STATUS}\nsecond line")
    );
}

#[test]
fn tap_for_another_agent_prints_nothing() {
    let sandbox = Sandbox::new();
    sandbox.write_claude_settings(r#"{"statusLine":{"type":"command","command":"echo mine"}}"#);

    let output = run_with_stdin(
        sandbox
            .orch()
            .args(["tap", "--agent", "antigravity", "--session", "gone"]),
        STATUS,
    );

    assert!(output.status.success());
    assert!(output.stdout.is_empty(), "{output:?}");
}

#[test]
fn tap_rereads_the_users_statusline_on_every_call() {
    let sandbox = Sandbox::new();
    let tap = || {
        let output = run_with_stdin(sandbox.orch().args(["tap", "--session", "gone"]), STATUS);
        String::from_utf8(output.stdout).unwrap()
    };
    sandbox.write_claude_settings(r#"{"statusLine":{"type":"command","command":"echo first"}}"#);
    assert_eq!(tap(), "first\n");

    sandbox.write_claude_settings(r#"{"statusLine":{"type":"command","command":"echo second"}}"#);
    assert_eq!(tap(), "second\n");

    sandbox.write_claude_settings("{}");
    assert_eq!(tap(), "Opus · 42%\n");
}

#[tokio::test]
async fn tap_forwards_the_statusline_payload_to_the_holder() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "");
    let (mut client, _) = held.attach().await;

    type_line(&mut client, &format!("tap {STATUS}")).await;

    assert_eq!(
        next_hook_or_tap(&mut client).await,
        HolderEvent::Tap {
            agent: Some(ClaudeCode::NAME.into()),
            payload: STATUS.into()
        }
    );
    wait_for_screen(&mut client, |text| text.contains("tap> Opus · 42%")).await;
}

#[test]
fn tap_prefers_the_worktrees_local_project_statusline() {
    let sandbox = Sandbox::new();
    sandbox.write_claude_settings(r#"{"statusLine":{"type":"command","command":"echo user"}}"#);
    sandbox.write_settings(
        sandbox.path("work/.claude/settings.json"),
        r#"{"statusLine":{"type":"command","command":"echo project"}}"#,
    );
    sandbox.write_settings(
        sandbox.path("work/.claude/settings.local.json"),
        r#"{"statusLine":{"type":"command","command":"echo local"}}"#,
    );

    let output = run_with_stdin(sandbox.orch().args(["tap", "--session", "gone"]), STATUS);

    assert_eq!(String::from_utf8(output.stdout).unwrap(), "local\n");
}

#[test]
fn a_hanging_user_statusline_falls_back_to_the_minimal_line() {
    let sandbox = Sandbox::new();
    sandbox.write_claude_settings(r#"{"statusLine":{"type":"command","command":"sleep 30"}}"#);
    let started = Instant::now();

    let output = run_with_stdin(sandbox.orch().args(["tap", "--session", "gone"]), STATUS);

    assert_eq!(String::from_utf8(output.stdout).unwrap(), "Opus · 42%\n");
    assert!(started.elapsed() < Duration::from_secs(8));
}

#[test]
fn large_statusline_input_and_output_do_not_deadlock() {
    let sandbox = Sandbox::new();
    sandbox.write_claude_settings(
        r#"{"statusLine":{"type":"command","command":"head -c 300000 /dev/zero | tr '\\0' x; cat > /dev/null"}}"#,
    );
    let payload = format!(r#"{{"padding":"{}"}}"#, "p".repeat(1 << 20));

    let output = run_with_stdin(sandbox.orch().args(["tap", "--session", "gone"]), &payload);

    assert_eq!(output.stdout.len(), 300_000);
}

#[test]
fn hook_with_an_invalid_session_id_still_exits_quietly() {
    let sandbox = Sandbox::new();

    let output = run_with_stdin(
        sandbox.orch().args(["hook", "--session", "../escape"]),
        r#"{"hook_event_name":"Stop"}"#,
    );

    assert!(output.status.success());
    assert!(output.stdout.is_empty());
}
