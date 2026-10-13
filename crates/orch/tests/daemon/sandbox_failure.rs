use orch_core::SessionId;
use orch_protocol::{AgentStateView as State, CreateSession, Reply, Request, RequestError};

use crate::common::*;

const PRESETS: &str =
    "[defaults.presets.boxed]\nsandbox = true\n\n[defaults.presets.open]\nsandbox = false\n";

fn refusal(reply: Result<Reply, RequestError>) -> String {
    match reply {
        Err(RequestError::Refused { message }) => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn boxed(env: &Env, prompt: &str) -> CreateSession {
    let repo = env.path("repos/app");
    if !repo.exists() {
        env.repo("app");
    }
    let mut create = CreateSession::new(&repo, prompt);
    create.preset = Some("boxed".into());
    create
}

async fn running_boxed(env: &Env, client: &mut TestClient) -> SessionId {
    let id = client.create(boxed(env, "Boxed")).await;
    client
        .until(&id, "running", |view| view.agent.is_some())
        .await;
    id
}

async fn set_preset(client: &mut TestClient, id: &SessionId, preset: &str) {
    let reply = client
        .request(Request::SetPreset {
            session: id.clone(),
            preset: preset.into(),
        })
        .await;
    assert_eq!(reply, Ok(Reply::Done));
}

fn sandboxed_env() -> Env {
    let env = Env::new();
    env.write_config(PRESETS);
    env.fake_bwrap();
    env
}

#[tokio::test]
async fn a_new_session_is_refused_before_its_worktree_when_the_sandbox_cannot_run() {
    let env = sandboxed_env();
    env.bwrap_switch("outer-fails", true);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    let create = boxed(&env, "Boxed");
    let message = refusal(client.request(Request::CreateSession(create)).await);

    assert!(message.starts_with("The Sandbox can't run"), "{message}");
    assert!(message.contains("setting up uid map"), "{message}");
    assert!(client.session_list().await.is_empty());
    let worktrees = git(&env.path("repos/app"), &["worktree", "list"]);
    assert_eq!(worktrees.lines().count(), 1, "{worktrees}");
}

#[tokio::test]
async fn resuming_into_a_sandbox_that_cannot_run_is_refused() {
    let env = sandboxed_env();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_boxed(&env, &mut client).await;
    let mut pane = env.pane(&id, PANE).await;
    pane.type_line("exit 0").await;
    client
        .until(&id, "Exited", |view| view.agent == Some(State::Exited))
        .await;

    env.bwrap_switch("outer-fails", true);
    let message = refusal(client.request(Request::Resume { session: id }).await);

    assert!(message.starts_with("The Sandbox can't run"), "{message}");
}

#[tokio::test]
async fn a_relaunch_into_a_broken_sandbox_errors_and_an_opted_out_preset_recovers() {
    let env = sandboxed_env();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = idle_session(&env, &mut client, "Open").await;
    env.bwrap_switch("outer-fails", true);

    set_preset(&mut client, &id, "boxed").await;
    let errored = client
        .until(&id, "Errored", |view| view.agent == Some(State::Errored))
        .await;
    let error = errored.error.unwrap_or_default();
    assert!(error.starts_with("The Sandbox can't run"), "{error}");

    set_preset(&mut client, &id, "open").await;
    let relaunched = client
        .until(&id, "running again", |view| {
            view.agent.is_some_and(|agent| agent != State::Errored)
        })
        .await;
    assert_eq!(relaunched.error, None);
    let mut pane = env.pane(&id, PANE).await;
    pane.type_line("print unboxed").await;
    pane.wait_for_text("unboxed").await;
}

#[tokio::test]
async fn a_broken_nested_sandbox_warns_in_the_pane_and_still_runs_the_agent() {
    let env = sandboxed_env();
    env.bwrap_switch("nested-fails", true);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_boxed(&env, &mut client).await;

    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text("Claude's own Bash sandbox won't work")
        .await;
    pane.type_line("print still-running").await;
    pane.wait_for_text("still-running").await;
}

#[tokio::test]
async fn a_sandbox_that_fails_to_start_errors_the_agent_with_what_bwrap_said() {
    let env = sandboxed_env();
    env.bwrap_switch("start-fails", true);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = client.create(boxed(&env, "Boxed")).await;

    let errored = client
        .until(&id, "Errored", |view| view.agent == Some(State::Errored))
        .await;
    assert_eq!(
        errored.error.as_deref(),
        Some("The Sandbox failed to start: bwrap: Can't find source path /gone")
    );
}
