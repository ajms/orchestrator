use orch_core::SessionId;
use orch_protocol::{
    AgentStateView as State, CreateSession, PhaseView, Reply, Request, RequestError,
};

use crate::common::*;

fn report_hook(env: &Env, id: &SessionId, payload: &str) {
    let mut command = env.orch();
    command
        .args(["hook", "--session", id.as_str()])
        .env("ORCH_HOLDER_SOCKET", env.holder_socket(id))
        .stdin(std::process::Stdio::piped());
    let mut child = command.spawn().unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
}

#[tokio::test]
async fn a_restarted_daemon_adopts_live_holders_and_their_buffered_events() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Survive").await;
    let mut pane = env.pane(&id, PANE).await;
    pane.type_line("print before the restart").await;
    pane.wait_for_text("before the restart").await;
    let holder = client.holder_pid(&id).unwrap();

    daemon.kill();
    report_hook(
        &env,
        &id,
        &hook("PermissionRequest", r#""tool_name":"Bash""#),
    );
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let adopted = client
        .until(&id, "Needs input", |view| {
            view.agent == Some(State::NeedsInput)
        })
        .await;
    assert_eq!(adopted.phase, PhaseView::Active);
    assert_eq!(adopted.holder_pid, Some(holder as u32));

    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text("before the restart").await;
    pane.hook(&hook("Stop", "")).await;
    client
        .until(&id, "Idle", |view| view.agent == Some(State::Idle))
        .await;
}

#[tokio::test]
async fn a_session_whose_holder_died_is_suspended_and_resumes_its_conversation() {
    let env = Env::new();
    let repo = env.repo("app");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let mut create = CreateSession::new(&repo, "Long task");
    create.preset = Some("edits".into());
    let id = client.create(create).await;
    client
        .until(&id, "running", |view| view.agent.is_some())
        .await;
    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text("permission-mode> acceptEdits").await;
    pane.hook(&hook("SessionStart", r#""permission_mode":"plan""#))
        .await;
    client
        .until(&id, "Idle in plan mode", |view| {
            view.agent == Some(State::Idle) && view.mode.as_deref() == Some("plan")
        })
        .await;

    kill(client.holder_pid(&id).unwrap(), "-KILL");
    let suspended = client
        .until(&id, "Suspended", |view| view.phase == PhaseView::Suspended)
        .await;
    assert_eq!(suspended.agent, None);
    assert_eq!(suspended.port_base, Some(20000));

    let resumed = client
        .request(Request::Resume {
            session: id.clone(),
        })
        .await;
    assert_eq!(resumed, Ok(Reply::Done));
    let active = client
        .until(&id, "Active again", |view| {
            view.phase == PhaseView::Active && view.agent.is_some()
        })
        .await;
    assert_eq!(active.port_base, Some(20000));
    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text("resume> conv-1").await;
    pane.wait_for_text("permission-mode> plan").await;
    assert!(!pane.text().contains("prompt>"));
    pane.type_line("env ORCH_PORT_BASE").await;
    pane.wait_for_text("ORCH_PORT_BASE=20000").await;
}

#[tokio::test]
async fn a_holder_that_died_while_the_daemon_was_down_leaves_its_session_suspended() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Reboot").await;
    let holder = client.holder_pid(&id).unwrap();
    daemon.kill();
    kill(holder, "-KILL");
    wait_until("the Holder to die", || is_zombie(holder)).await;

    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let listed = client.session_list().await;
    assert_eq!(listed[0].phase, PhaseView::Suspended);
}

#[tokio::test]
async fn an_exited_agent_keeps_its_holder_until_resumed_into_a_fresh_one() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Finish").await;
    let mut pane = env.pane(&id, PANE).await;

    let early = client
        .request(Request::Resume {
            session: id.clone(),
        })
        .await;
    assert!(
        matches!(early, Err(RequestError::Refused { .. })),
        "{early:?}"
    );

    pane.hook(&hook("SessionStart", "")).await;
    pane.type_line("exit 0").await;
    client
        .until(&id, "Exited", |view| view.agent == Some(State::Exited))
        .await;
    let old_holder = client.holder_pid(&id).unwrap() as u32;
    client.history.clear();

    client
        .request(Request::Resume {
            session: id.clone(),
        })
        .await
        .unwrap();
    client
        .until(&id, "running in a fresh Holder", |view| {
            view.holder_pid.is_some_and(|pid| pid != old_holder)
                && matches!(view.agent, Some(State::Starting | State::Working))
        })
        .await;
    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text("resume> conv-1").await;
    assert!(
        client
            .history
            .iter()
            .all(|view| view.phase == PhaseView::Active),
        "resuming replaces the Holder without suspending the Session"
    );
}

#[tokio::test]
async fn an_idle_agent_is_still_idle_after_a_daemon_restart_without_new_events() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Rest").await;
    let mut pane = env.pane(&id, PANE).await;
    pane.hook(&hook("SessionStart", r#""permission_mode":"plan""#))
        .await;
    client
        .until(&id, "Idle in plan mode", |view| {
            view.agent == Some(State::Idle) && view.mode.as_deref() == Some("plan")
        })
        .await;
    settled(&mut client).await;

    daemon.kill();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let listed = client.session_list().await;
    assert_eq!(listed[0].agent, Some(State::Idle));
    assert_eq!(listed[0].mode.as_deref(), Some("plan"));
    assert_eq!(listed[0].phase, PhaseView::Active);
}

#[tokio::test]
async fn resuming_after_a_failed_launch_starts_fresh_with_the_initial_prompt() {
    let env = Env::new();
    let repo = env.repo("app");
    env.write_config_with_agent("", "/nonexistent/agent");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = client.create(CreateSession::new(&repo, "Try again")).await;
    let failed = client
        .until(&id, "a failed launch", |view| view.error.is_some())
        .await;
    assert_eq!(failed.agent, Some(State::Errored));
    assert_eq!(failed.holder_pid, None);

    env.write_config("");
    let resumed = client
        .request(Request::Resume {
            session: id.clone(),
        })
        .await;
    assert_eq!(resumed, Ok(Reply::Done));
    client
        .until(&id, "running", |view| view.holder_pid.is_some())
        .await;
    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text("prompt> Try again").await;
    pane.wait_for_text(&format!("session-id> {}", id.as_str()))
        .await;
}

#[tokio::test]
async fn a_holder_connection_taken_over_transiently_is_reattached_not_suspended() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Steady").await;
    client.history.clear();

    let mut intruder = orch_holder::HolderClient::connect(&env.holder_socket(&id))
        .await
        .unwrap();
    intruder.attach().await.unwrap();
    loop {
        let message = tokio::time::timeout(WAIT, intruder.recv())
            .await
            .expect("the Daemon never took the Holder back")
            .unwrap();
        match message {
            Some(orch_holder::FromHolder::Superseded) | None => break,
            Some(_) => {}
        }
    }

    let mut pane = env.pane(&id, PANE).await;
    pane.hook(&hook("SessionStart", "")).await;
    client
        .until(&id, "Idle", |view| view.agent == Some(State::Idle))
        .await;
    assert!(
        client
            .history
            .iter()
            .all(|view| view.phase == PhaseView::Active)
    );
}
