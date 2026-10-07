use orch_agent::{ClaudeCode, GuardAnswer};
use orch_holder::{HolderEvent, ToHolder};
use serde_json::Value;

use crate::common::*;

const PRE_TOOL_USE: &str = r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"git checkout main"}}"#;

async fn guard_request(client: &mut orch_holder::HolderClient) -> u64 {
    type_line(client, &format!("hook {PRE_TOOL_USE}")).await;
    match next_hook_or_tap(client).await {
        HolderEvent::Hook {
            payload,
            guard: Some(id),
            ..
        } => {
            assert_eq!(payload, PRE_TOOL_USE);
            id
        }
        other => panic!("expected a waiting guard, got {other:?}"),
    }
}

fn decision(text: &str) -> Option<String> {
    let line = text.lines().find_map(|line| line.strip_prefix("hook> "))?;
    let output: Value = serde_json::from_str(line).ok()?;
    output["hookSpecificOutput"]["permissionDecision"]
        .as_str()
        .map(String::from)
}

async fn screen_decision(client: &mut orch_holder::HolderClient) -> Option<String> {
    let snapshot = wait_for_screen(client, |text| text.contains("hook>")).await;
    decision(&snapshot.text())
}

#[tokio::test]
async fn daemon_allows_a_guarded_tool_call() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "");
    let (mut client, _) = held.attach().await;
    let id = guard_request(&mut client).await;

    client
        .send(&ToHolder::GuardAnswer {
            id,
            answer: GuardAnswer::Proceed,
        })
        .await
        .unwrap();

    let snapshot = wait_for_screen(&mut client, |text| text.contains("hook>")).await;
    assert!(
        snapshot
            .text()
            .lines()
            .any(|line| line.trim_end() == "hook>")
    );
}

#[tokio::test]
async fn daemon_denies_a_guarded_tool_call_with_a_reason() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "");
    let (mut client, _) = held.attach().await;
    let id = guard_request(&mut client).await;

    client
        .send(&ToHolder::GuardAnswer {
            id,
            answer: GuardAnswer::Deny {
                reason: "touches the Base branch".into(),
            },
        })
        .await
        .unwrap();

    let snapshot = wait_for_screen(&mut client, |text| text.contains("hook>")).await;
    assert_eq!(decision(&snapshot.text()).as_deref(), Some("deny"));
    assert!(snapshot.text().contains("touches the Base branch"));
}

#[tokio::test]
async fn guard_without_a_daemon_asks_the_user_and_is_still_recorded() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", &format!("hook {PRE_TOOL_USE}\n"));
    let mut observer = orch_holder::HolderClient::connect(&held.socket)
        .await
        .unwrap();

    assert_eq!(screen_decision(&mut observer).await.as_deref(), Some("ask"));

    let (mut client, _) = held.attach().await;
    assert_eq!(
        next_hook_or_tap(&mut client).await,
        HolderEvent::Hook {
            agent: Some(ClaudeCode::NAME.into()),
            payload: PRE_TOOL_USE.into(),
            guard: None
        }
    );
}

#[tokio::test]
async fn agy_is_told_to_force_its_own_prompt_when_no_daemon_answers_the_guard() {
    let sandbox = Sandbox::new();
    let tool_call = r#"{"toolCall":{"name":"run_command","args":{"CommandLine":"git checkout main"}},"stepIdx":3}"#;
    let held = sandbox.hold(
        "s1",
        &format!("hook --agent antigravity --event PreToolUse {tool_call}\n"),
    );
    let mut observer = orch_holder::HolderClient::connect(&held.socket)
        .await
        .unwrap();

    let snapshot = wait_for_screen(&mut observer, |text| text.contains("hook>")).await;
    assert!(
        snapshot
            .text()
            .contains(r#"hook> {"decision":"force_ask"}"#),
        "{}",
        snapshot.text()
    );
}

#[tokio::test]
async fn unanswered_guard_times_out_to_ask() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold_with("s1", "", &["--guard-timeout-ms", "300"]);
    let (mut client, _) = held.attach().await;
    guard_request(&mut client).await;

    assert_eq!(screen_decision(&mut client).await.as_deref(), Some("ask"));
}

#[tokio::test]
async fn held_guard_waits_past_the_timeout_for_the_users_answer() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold_with("s1", "", &["--guard-timeout-ms", "300"]);
    let (mut client, _) = held.attach().await;
    let id = guard_request(&mut client).await;

    client.send(&ToHolder::GuardHeld { id }).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    assert!(!client.snapshot().await.unwrap().text().contains("hook>"));
    client
        .send(&ToHolder::GuardAnswer {
            id,
            answer: GuardAnswer::Deny {
                reason: "no".into(),
            },
        })
        .await
        .unwrap();

    assert_eq!(screen_decision(&mut client).await.as_deref(), Some("deny"));
}

#[tokio::test]
async fn daemon_disconnect_turns_pending_guards_into_ask() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "");
    let (mut client, _) = held.attach().await;
    guard_request(&mut client).await;

    drop(client);

    let mut observer = orch_holder::HolderClient::connect(&held.socket)
        .await
        .unwrap();
    assert_eq!(screen_decision(&mut observer).await.as_deref(), Some("ask"));
}
