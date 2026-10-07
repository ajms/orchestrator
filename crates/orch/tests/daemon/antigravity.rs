use orch_agent::Antigravity;
use orch_core::SessionId;
use orch_protocol::{
    AgentStateView as State, AgentUsageWindows, CreateSession, PhaseView, Reply, Request,
    UsageWindowView,
};
use serde_json::{Value, json};

use crate::cli::orch;
use crate::common::*;

const CONVERSATION: &str = "3c1e9a40-7d52-4b8e-a6f1-2d9b0c4e7a13";

async fn antigravity_session(
    env: &Env,
    client: &mut TestClient,
    preset: &str,
) -> (SessionId, PaneView) {
    env.write_config(&format!(
        "[defaults.agents.antigravity]\nbinary = {:?}\nargs = [\"fake-agent\", \"--\"]\n",
        env!("CARGO_BIN_EXE_orch")
    ));
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

async fn hook(pane: &mut PaneView, event: &str, fields: Value) {
    let mut payload = json!({ "conversationId": CONVERSATION });
    payload
        .as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    pane.type_line(&format!(
        "hook --agent antigravity --event {event} {payload}"
    ))
    .await;
}

async fn tap(pane: &mut PaneView, fields: Value) {
    pane.type_line(&format!("tap --agent antigravity {}", line(fields)))
        .await;
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
