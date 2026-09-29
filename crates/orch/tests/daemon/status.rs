use orch_protocol::{AgentStateView as State, Request, SessionView};

use crate::common::*;

fn agent_is(state: State) -> impl Fn(&SessionView) -> bool {
    move |view| view.agent == Some(state)
}

#[tokio::test]
async fn the_agent_state_follows_the_agents_hooks_and_exit() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Refactor").await;
    let mut pane = env.pane(&id, PANE).await;

    pane.hook(&hook("SessionStart", r#""source":"startup""#))
        .await;
    let idle = client.until(&id, "Idle", agent_is(State::Idle)).await;
    assert_eq!(idle.conversation.as_deref(), Some("conv-1"));

    pane.hook(&hook("UserPromptSubmit", r#""permission_mode":"plan""#))
        .await;
    client
        .until(&id, "Working in plan mode", |view| {
            view.agent == Some(State::Working) && view.mode.as_deref() == Some("plan")
        })
        .await;

    pane.hook(&hook("PermissionRequest", r#""tool_name":"Bash""#))
        .await;
    client
        .until(&id, "Needs input", agent_is(State::NeedsInput))
        .await;

    pane.hook(&hook("Stop", "")).await;
    client.until(&id, "Idle", agent_is(State::Idle)).await;

    pane.hook(&hook("StopFailure", r#""error":"rate_limit""#))
        .await;
    client.until(&id, "Errored", agent_is(State::Errored)).await;

    pane.type_line("exit 0").await;
    let exited = client.until(&id, "Exited", agent_is(State::Exited)).await;
    assert_eq!(exited.phase, orch_protocol::PhaseView::Active);
    pane.wait_for_text("prompt> Refactor").await;
}

#[tokio::test]
async fn an_agent_exiting_with_failure_is_errored_and_keeps_its_final_screen() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Crash").await;
    let mut pane = env.pane(&id, PANE).await;
    pane.type_line("print last words").await;
    pane.type_line("exit 3").await;
    client.until(&id, "Errored", agent_is(State::Errored)).await;

    let mut reopened = env.pane(&id, PANE).await;
    reopened.wait_for_text("last words").await;
}

#[tokio::test]
async fn a_finished_turn_marks_the_session_unseen_only_while_nobody_watches_it() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Watch me").await;
    let mut pane = env.pane(&id, PANE).await;

    client
        .request(Request::View {
            session: Some(id.clone()),
            focused: true,
        })
        .await
        .unwrap();
    pane.hook(&hook("Stop", "")).await;
    let watched = client.until(&id, "Idle", agent_is(State::Idle)).await;
    assert!(!watched.flags.unseen);

    client
        .request(Request::View {
            session: Some(id.clone()),
            focused: false,
        })
        .await
        .unwrap();
    pane.hook(&hook("UserPromptSubmit", "")).await;
    client.until(&id, "Working", agent_is(State::Working)).await;
    pane.hook(&hook("Stop", "")).await;
    let unseen = client.until(&id, "Unseen", |view| view.flags.unseen).await;
    assert_eq!(unseen.agent, Some(State::Idle));

    client
        .request(Request::View {
            session: Some(id.clone()),
            focused: true,
        })
        .await
        .unwrap();
    client.until(&id, "seen", |view| !view.flags.unseen).await;
}

#[tokio::test]
async fn a_working_session_without_activity_is_flagged_stalled_until_it_acts_again() {
    let env = Env::new();
    env.write_config("stalled_minutes = 0\n");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Hang").await;
    let mut pane = env.pane(&id, PANE).await;

    pane.hook(&hook("UserPromptSubmit", "")).await;
    let stalled = client
        .until(&id, "Stalled", |view| view.flags.stalled)
        .await;
    assert_eq!(stalled.agent, Some(State::Working));
    pane.hook(&hook("Stop", "")).await;
    client
        .until(&id, "Idle and not stalled", |view| {
            view.agent == Some(State::Idle) && !view.flags.stalled
        })
        .await;
}
