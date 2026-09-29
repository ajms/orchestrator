use orch_core::SessionId;
use orch_protocol::{AgentStateView as State, Reply, Request, RequestError};

use crate::common::*;

async fn set_preset(
    client: &mut TestClient,
    id: &SessionId,
    preset: &str,
) -> Result<Reply, RequestError> {
    client
        .request(Request::SetPreset {
            session: id.clone(),
            preset: preset.into(),
        })
        .await
}

#[tokio::test]
async fn changing_the_preset_resumes_the_conversation_with_the_new_flags() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut client, "Tighten").await;
    pane.hook(&hook("UserPromptSubmit", r#""permission_mode":"default""#))
        .await;
    pane.hook(&hook("Stop", "")).await;
    client
        .until(&id, "Idle in default mode", |view| {
            view.agent == Some(State::Idle) && view.mode.as_deref() == Some("default")
        })
        .await;
    let before = client.sessions[&id].clone();
    let old_holder = client.holder_pid(&id).unwrap();

    assert_eq!(set_preset(&mut client, &id, "plan").await, Ok(Reply::Done));

    let after = client
        .until(&id, "restarted", |view| {
            view.holder_pid.is_some_and(|pid| pid as i32 != old_holder) && view.agent.is_some()
        })
        .await;
    assert_eq!(after.preset, "plan");
    assert_eq!(after.port_base, before.port_base);
    assert_eq!(after.conversation.as_deref(), Some("conv-1"));
    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text("resume> conv-1").await;
    pane.wait_for_text("permission-mode> plan").await;
    wait_until("the old Holder to exit", || process_gone(old_holder)).await;
}

#[tokio::test]
async fn the_preset_cannot_change_while_the_agent_works_or_to_an_unknown_preset() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut client, "Busy").await;

    assert!(matches!(
        set_preset(&mut client, &id, "nonsense").await,
        Err(RequestError::Refused { message }) if message.contains("nonsense")
    ));

    pane.hook(&hook("UserPromptSubmit", "")).await;
    client
        .until(&id, "Working", |view| view.agent == Some(State::Working))
        .await;
    assert!(matches!(
        set_preset(&mut client, &id, "plan").await,
        Err(RequestError::Refused { message }) if message.contains("Working")
    ));
    settled(&mut client).await;
    assert_eq!(client.sessions[&id].preset, "inherit");
}

#[tokio::test]
async fn muting_a_session_is_remembered_across_a_daemon_restart() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Quiet").await;

    let muted = client
        .request(Request::SetMuted {
            session: id.clone(),
            muted: true,
        })
        .await;
    assert_eq!(muted, Ok(Reply::Done));
    client.until(&id, "Muted", |view| view.flags.muted).await;

    daemon.kill();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    assert!(client.session_list().await[0].flags.muted);
    client
        .request(Request::SetMuted {
            session: id.clone(),
            muted: false,
        })
        .await
        .unwrap();
    client.until(&id, "unmuted", |view| !view.flags.muted).await;
}
