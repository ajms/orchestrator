use std::path::PathBuf;
use std::time::Duration;

use orch_agent::Antigravity;
use orch_core::SessionId;
use orch_holder::SESSION_ENV;
use orch_protocol::{
    AgentStateView as State, AgentUsageWindows, CreateSession, GuardChoice, LandingMode, PhaseView,
    Reply, Request, UsageWindowView,
};
use serde_json::{Value, json};

use crate::cli::orch;
use crate::common::*;

const CONVERSATION: &str = "3c1e9a40-7d52-4b8e-a6f1-2d9b0c4e7a13";
const CHILD: &str = "8d2f61b7-4a09-4c3e-9b15-e07a3c5d9f28";

async fn antigravity_session(
    env: &Env,
    client: &mut TestClient,
    preset: &str,
) -> (SessionId, PaneView) {
    antigravity_session_with(env, client, preset, "").await
}

async fn antigravity_session_with(
    env: &Env,
    client: &mut TestClient,
    preset: &str,
    config: &str,
) -> (SessionId, PaneView) {
    scripted_antigravity_session(env, client, preset, config, &[]).await
}

fn write_agy_config(env: &Env, config: &str, lines: &[String]) {
    let script = env.path("agy.script");
    std::fs::write(&script, format!("draft -p\n{}", lines.join("\n"))).unwrap();
    env.write_config(&format!(
        "[defaults.agents.antigravity]\nbinary = {:?}\nargs = [\"fake-agent\", \"--script\", {script:?}, \"--\"]\n{config}",
        env!("CARGO_BIN_EXE_orch")
    ));
}

async fn scripted_antigravity_session(
    env: &Env,
    client: &mut TestClient,
    preset: &str,
    config: &str,
    lines: &[String],
) -> (SessionId, PaneView) {
    write_agy_config(env, config, lines);
    let installed = orch(env, &["agent", "install", "antigravity", "--yes"]).await;
    assert!(installed.status.success(), "{installed:?}");
    let repo = env.repo("app");
    let mut create = CreateSession::new(&repo, "Fix the login bug");
    create.agent = Some("antigravity".into());
    create.preset = Some(preset.into());
    let id = client.create(create).await;
    client
        .until(&id, "running", |view| view.agent.is_some())
        .await;
    let pane = env.pane(&id, PANE).await;
    (id, pane)
}

fn line(fields: Value) -> String {
    let mut line = json!({ "product": "antigravity", "conversation_id": CONVERSATION });
    line.as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    line.to_string()
}

fn hook_line(event: &str, fields: Value) -> String {
    let mut payload = json!({ "conversationId": CONVERSATION });
    payload
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    format!("hook --agent antigravity --event {event} {payload}")
}

fn tap_line(fields: Value) -> String {
    format!("tap --agent antigravity {}", line(fields))
}

async fn hook(pane: &mut PaneView, event: &str, fields: Value) {
    pane.type_line(&hook_line(event, fields)).await;
}

async fn tap(pane: &mut PaneView, fields: Value) {
    pane.type_line(&tap_line(fields)).await;
}

async fn until_state(client: &mut TestClient, id: &SessionId, state: State) {
    client
        .until(id, &format!("{state:?}"), |view| view.agent == Some(state))
        .await;
}

async fn working(client: &mut TestClient, id: &SessionId, pane: &mut PaneView) {
    hook(
        pane,
        "PreInvocation",
        json!({ "invocationNum": 1, "initialNumSteps": 1 }),
    )
    .await;
    until_state(client, id, State::Working).await;
}

#[tokio::test]
async fn agy_launches_with_its_prompt_and_waits_on_the_trust_screen_until_it_is_ready() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = antigravity_session(&env, &mut client, "plan").await;
    pane.wait_for_text("i> Fix the login bug").await;
    pane.wait_for_text("mode> plan").await;

    tap(
        &mut pane,
        json!({ "conversation_id": "", "agent_state": "initializing", "tool_confirmation_pending": true }),
    )
    .await;
    until_state(&mut client, &id, State::NeedsInput).await;

    tap(&mut pane, json!({ "agent_state": "idle" })).await;
    until_state(&mut client, &id, State::Idle).await;
}

#[tokio::test]
async fn a_pending_tool_confirmation_needs_input_until_the_tool_finishes() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = antigravity_session(&env, &mut client, "edits").await;
    working(&mut client, &id, &mut pane).await;

    tap(
        &mut pane,
        json!({ "agent_state": "tool_use", "tool_confirmation_pending": true }),
    )
    .await;
    until_state(&mut client, &id, State::NeedsInput).await;

    hook(
        &mut pane,
        "PostToolUse",
        json!({ "stepIdx": 4, "error": "" }),
    )
    .await;
    until_state(&mut client, &id, State::Working).await;
}

#[tokio::test]
async fn only_a_fully_idle_stop_makes_the_session_idle() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = antigravity_session(&env, &mut client, "edits").await;
    working(&mut client, &id, &mut pane).await;
    let stop = |fully_idle: bool| json!({ "executionNum": 1, "terminationReason": "NO_TOOL_CALL", "error": "", "fullyIdle": fully_idle });

    hook(&mut pane, "Stop", stop(false)).await;
    tap(
        &mut pane,
        json!({ "agent_state": "working", "cycle_mode": "plan" }),
    )
    .await;
    let waiting = client
        .until(&id, "the mode after the Stop", |view| {
            view.mode.as_deref() == Some("plan")
        })
        .await;
    assert_eq!(waiting.agent, Some(State::Working));

    hook(&mut pane, "Stop", stop(true)).await;
    until_state(&mut client, &id, State::Idle).await;
}

#[tokio::test]
async fn another_conversation_under_the_session_is_a_subagent_whose_row_reopens() {
    const PROMPT: &str = "Run the login tests and report the failures.";
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = antigravity_session(&env, &mut client, "edits").await;
    tap(&mut pane, json!({ "agent_state": "working" })).await;
    let spec = json!({ "TypeName": "general", "Role": "Test Runner", "Prompt": PROMPT });
    hook(
        &mut pane,
        "PreToolUse",
        json!({ "toolCall": { "name": "invoke_subagent", "args": { "Subagents": [spec] } } }),
    )
    .await;

    let transcript = env.path("brain/child/transcript_full.jsonl");
    std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    let message = format!(
        "<SYSTEM_MESSAGE>\n[Message] timestamp=2026-10-07T08:00:05Z sender={CONVERSATION} priority=MESSAGE_PRIORITY_HIGH content={PROMPT}\n</SYSTEM_MESSAGE>"
    );
    let step =
        json!({ "step_index": 0, "type": "SYSTEM_MESSAGE", "status": "DONE", "content": message });
    std::fs::write(&transcript, format!("{step}\n")).unwrap();
    let child = |fields: Value| {
        let mut payload = json!({ "conversationId": CHILD, "transcriptPath": transcript });
        payload
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        payload
    };
    let row = |view: &orch_protocol::SessionView| view.subagents.first().cloned();

    hook(&mut pane, "PreInvocation", child(json!({}))).await;
    hook(
        &mut pane,
        "PreToolUse",
        child(json!({ "toolCall": { "name": "run_command", "args": { "CommandLine": "cargo test" } } })),
    )
    .await;
    let running = client
        .until(&id, "the Subagent's tool", |view| {
            row(view).is_some_and(|row| row.tool_count == 1)
        })
        .await;
    let started = row(&running).unwrap();
    assert_eq!(
        (started.id.as_str(), started.agent_type.as_str()),
        (CHILD, "general")
    );
    assert_eq!(started.description, "Test Runner");

    hook(
        &mut pane,
        "Stop",
        child(json!({ "terminationReason": "NO_TOOL_CALL", "fullyIdle": false })),
    )
    .await;
    client
        .until(&id, "the Subagent done", |view| {
            row(view).is_some_and(|row| row.done)
        })
        .await;

    hook(&mut pane, "PreInvocation", child(json!({}))).await;
    let reopened = client
        .until(&id, "the Subagent running again", |view| {
            row(view).is_some_and(|row| !row.done)
        })
        .await;
    assert_eq!(reopened.subagents.len(), 1);
    assert_eq!(reopened.conversation.as_deref(), Some(CONVERSATION));

    let subscribe = Request::SubscribeSubagent {
        session: id.clone(),
        subagent: CHILD.into(),
    };
    client.request(subscribe).await.unwrap();
    client
        .until_received("the Subagent transcript", |client| {
            !client.transcripts.is_empty()
        })
        .await;
    assert_eq!(
        client.transcripts[0].entries,
        [orch_core::TranscriptEntry::Prompt {
            text: PROMPT.into()
        }]
    );
}

#[tokio::test]
async fn a_mode_cycled_in_agy_is_kept_when_the_session_resumes() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = antigravity_session(&env, &mut client, "edits").await;
    pane.wait_for_text("mode> accept-edits").await;
    tap(
        &mut pane,
        json!({ "agent_state": "idle", "cycle_mode": "plan" }),
    )
    .await;
    client
        .until(&id, "plan mode", |view| {
            view.mode.as_deref() == Some("plan")
        })
        .await;

    kill(client.holder_pid(&id).unwrap(), "-KILL");
    client
        .until(&id, "Suspended", |view| view.phase == PhaseView::Suspended)
        .await;
    let resumed = client
        .request(Request::Resume {
            session: id.clone(),
        })
        .await;
    assert_eq!(resumed, Ok(Reply::Done));

    client
        .until(&id, "Active again", |view| {
            view.phase == PhaseView::Active && view.agent.is_some()
        })
        .await;
    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text(&format!("conversation> {CONVERSATION}"))
        .await;
    pane.wait_for_text("mode> plan").await;
    assert!(!pane.text().contains("i>"));
}

#[tokio::test]
async fn each_agy_quota_pool_shows_as_a_usage_window_of_the_agent() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (_id, mut pane) = antigravity_session(&env, &mut client, "edits").await;

    tap(
        &mut pane,
        json!({ "agent_state": "idle", "quota": {
            "gemini-weekly": { "remaining_fraction": 0.25, "reset_time": "2026-10-12T00:00:00Z" },
            "3p-weekly": { "reset_time": "2026-10-12T00:00:00Z" },
        } }),
    )
    .await;
    client
        .until_received("usage windows", |client| !client.usage_windows.is_empty())
        .await;

    let window = |name: &str, label: &str, used_percent| UsageWindowView {
        name: name.into(),
        label: label.into(),
        used_percent,
        resets_at_unix: Some(1_791_763_200),
    };
    assert_eq!(
        client.usage_windows,
        [vec![AgentUsageWindows {
            agent: Antigravity::NAME.into(),
            windows: vec![
                window("gemini-weekly", "gemini-wk", 75.0),
                window("3p-weekly", "3p-wk", 100.0),
            ],
        }]]
    );
}

async fn idle_antigravity_session(env: &Env) -> (TestClient, SessionId, PaneView, PathBuf) {
    let mut client = env.client().await;
    let (id, mut pane) = antigravity_session(env, &mut client, "edits").await;
    tap(&mut pane, json!({ "agent_state": "idle" })).await;
    until_state(&mut client, &id, State::Idle).await;
    let worktree = client.sessions[&id].worktree.clone();
    (client, id, pane, worktree)
}

async fn drafted(client: &mut TestClient, id: &SessionId) -> (String, String) {
    let drafted = client
        .request(Request::Draft {
            session: id.clone(),
            mode: LandingMode::Squash,
        })
        .await;
    let Ok(Reply::Drafted { title, body }) = drafted else {
        panic!("no draft: {drafted:?}");
    };
    (title, body)
}

#[tokio::test]
async fn a_huge_diff_reaches_agy_truncated_after_a_stat_of_every_changed_file() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let (mut client, id, _pane, worktree) = idle_antigravity_session(&env).await;
    let huge: String = (0..4000)
        .map(|i| format!("generated line {i:>40}\n"))
        .collect();
    std::fs::write(worktree.join("generated.txt"), huge).unwrap();
    std::fs::write(worktree.join("small.txt"), "small line\n").unwrap();

    let (_, body) = drafted(&mut client, &id).await;

    let stat = |file: &str, lines: &str| {
        body.lines()
            .any(|line| line.contains(file) && line.contains(&format!("| {lines} +")))
    };
    assert!(stat("generated.txt", "4000"), "{body}");
    assert!(stat("small.txt", "   1"), "{body}");
    assert!(body.contains("diff truncated"), "{body}");
    assert!(!body.contains("generated line 3999"));
    assert!(body.len() < 110 * 1024, "{}", body.len());
}

#[tokio::test]
async fn agy_drafts_in_a_fresh_headless_run_fed_the_whole_diff_against_the_base() {
    let env = Env::new();
    let mut daemon = env.daemon_command(Duration::from_secs(600));
    daemon.env(SESSION_ENV, "outer-session");
    let _daemon = env.spawn_daemon(&mut daemon).await;
    let (mut client, id, pane, worktree) = idle_antigravity_session(&env).await;
    commit(&worktree, "committed.txt", "committed line\n");
    std::fs::write(worktree.join("README.md"), "uncommitted line\n").unwrap();
    std::fs::write(worktree.join("untracked.txt"), "untracked line\n").unwrap();

    let (title, body) = drafted(&mut client, &id).await;

    assert_eq!(title, "Drafted from nothing");
    assert!(
        body.starts_with(&format!("args: -p\n{SESSION_ENV}=<unset>\n")),
        "{body}"
    );
    assert!(body.contains("commit message"), "{body}");
    for change in ["+committed line", "+uncommitted line", "+untracked line"] {
        assert!(body.contains(change), "{change} missing from {body}");
    }
    assert!(!pane.text().contains("Drafted"));
}

async fn tool_call(pane: &mut PaneView, name: &str, args: Value) {
    hook(
        pane,
        "PreToolUse",
        json!({ "toolCall": { "name": name, "args": args }, "stepIdx": 9 }),
    )
    .await;
}

async fn guard_prompt(client: &mut TestClient, id: &SessionId) -> u64 {
    client
        .until(id, "a Guard prompt", |view| !view.guard_prompts.is_empty())
        .await
        .guard_prompts[0]
        .id
}

async fn answer_guard(client: &mut TestClient, id: &SessionId, guard: u64, choice: GuardChoice) {
    let reply = client
        .request(Request::AnswerGuard {
            session: id.clone(),
            guard,
            choice,
        })
        .await;
    assert_eq!(reply, Ok(Reply::Done));
}

const TIGHT: &str = "[defaults.presets.tight]\nmode = \"default\"\n[defaults.presets.tight.antigravity]\nallow = [\"command(npm test)\"]\ndeny = [\"command(rm)\"]\n";

#[tokio::test]
async fn a_guard_hit_waits_for_the_user_and_answers_agy_with_ask_or_deny() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = antigravity_session(&env, &mut client, "edits").await;
    let outside = || json!({ "TargetFile": "/etc/hosts", "CodeContent": "x" });

    tool_call(&mut pane, "write_to_file", outside()).await;
    let guard = guard_prompt(&mut client, &id).await;
    answer_guard(&mut client, &id, guard, GuardChoice::AllowOnce).await;
    pane.wait_for_text(r#"hook> {"decision":"ask"}"#).await;
    client
        .until(&id, "no Guard prompt", |view| view.guard_prompts.is_empty())
        .await;

    tool_call(&mut pane, "write_to_file", outside()).await;
    let guard = guard_prompt(&mut client, &id).await;
    answer_guard(&mut client, &id, guard, GuardChoice::Deny).await;
    pane.wait_for_text(r#"hook> {"decision":"deny""#).await;
}

#[tokio::test]
async fn preset_rules_deny_and_allow_agys_tool_calls() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = antigravity_session_with(&env, &mut client, "tight", TIGHT).await;
    let worktree = client.sessions[&id].worktree.clone();
    let command = |line: &str| json!({ "CommandLine": line, "Cwd": worktree });

    tool_call(&mut pane, "run_command", command("rm -rf build")).await;
    pane.wait_for_text(r#"hook> {"decision":"deny""#).await;

    tool_call(&mut pane, "run_command", command("npm test")).await;
    pane.wait_for_text(r#"hook> {"decision":"allow"}"#).await;

    tool_call(&mut pane, "run_command", command("make")).await;
    pane.wait_for_text(r#"hook> {"decision":"ask"}"#).await;

    let subagent_rm = json!({
        "conversationId": "8d2f61b7-4a09-4c3e-9b15-e07a3c5d9f28",
        "toolCall": { "name": "run_command", "args": command("rm -rf dist") },
        "stepIdx": 2,
    });
    pane.type_line(&format!(
        "hook --agent antigravity --event PreToolUse {subagent_rm}"
    ))
    .await;
    client
        .until(&id, "the Subagent", |view| !view.subagents.is_empty())
        .await;
    pane.type_line("print subagent-answered").await;
    pane.wait_for_text("subagent-answered").await;
    assert_eq!(pane.text().matches(r#"{"decision":"deny""#).count(), 2);
    assert!(
        client
            .history
            .iter()
            .all(|view| view.guard_prompts.is_empty())
    );
}

#[tokio::test]
async fn preset_rules_still_apply_to_a_running_agy_after_a_daemon_restart() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _) = antigravity_session_with(&env, &mut client, "tight", TIGHT).await;

    daemon.kill();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    client
        .until(&id, "the Agent again", |view| view.agent.is_some())
        .await;
    let mut pane = env.pane(&id, PANE).await;
    let rm = json!({ "CommandLine": "rm -rf build" });
    tool_call(&mut pane, "run_command", rm).await;
    pane.wait_for_text(r#"hook> {"decision":"deny""#).await;
}

#[tokio::test]
async fn a_subagent_starting_while_agy_waits_on_a_permission_keeps_the_session_needing_input() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let script = [
        hook_line("PreInvocation", json!({})),
        tap_line(json!({ "agent_state": "tool_use", "tool_confirmation_pending": true })),
        hook_line("PreInvocation", json!({ "conversationId": CHILD })),
    ];
    let (id, _) = scripted_antigravity_session(&env, &mut client, "edits", "", &script).await;

    let started = client
        .until(&id, "the Subagent", |view| !view.subagents.is_empty())
        .await;
    assert_eq!(started.agent, Some(State::NeedsInput));
}

#[tokio::test]
async fn an_adopted_agy_keeps_its_conversation_when_a_subagent_hooks_first() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = antigravity_session(&env, &mut client, "edits").await;
    working(&mut client, &id, &mut pane).await;
    tap(
        &mut pane,
        json!({ "conversation_id": "", "agent_state": "working", "cycle_mode": "plan" }),
    )
    .await;
    let known = client
        .until(&id, "the tap after the hook", |view| {
            view.mode.as_deref() == Some("plan")
        })
        .await;
    assert_eq!(known.conversation.as_deref(), Some(CONVERSATION));
    daemon.kill();
    let db = rusqlite::Connection::open(env.path("state/orchestrator/state.db")).unwrap();
    db.execute("UPDATE sessions SET agent_state = NULL", [])
        .unwrap();
    drop(db);

    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    client
        .until(&id, "the Agent again", |view| view.agent.is_some())
        .await;
    let mut pane = env.pane(&id, PANE).await;
    hook(
        &mut pane,
        "PreInvocation",
        json!({ "conversationId": CHILD }),
    )
    .await;
    let adopted = client
        .until(&id, "the Subagent's hook", |view| {
            !view.subagents.is_empty() || view.conversation.as_deref() != Some(CONVERSATION)
        })
        .await;
    assert_eq!(adopted.conversation.as_deref(), Some(CONVERSATION));
    assert_eq!(adopted.subagents[0].id, CHILD);
}

#[tokio::test]
async fn agy_must_ask_the_user_while_a_restarted_daemon_cannot_load_the_sessions_preset() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _) = antigravity_session_with(&env, &mut client, "tight", TIGHT).await;

    daemon.kill();
    write_agy_config(&env, "", &[]);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    client
        .until(&id, "the Agent again", |view| view.agent.is_some())
        .await;
    let mut pane = env.pane(&id, PANE).await;
    let npm = json!({ "CommandLine": "npm test" });
    tool_call(&mut pane, "run_command", npm).await;
    pane.wait_for_text(r#"hook> {"decision":"force_ask"}"#)
        .await;
}
