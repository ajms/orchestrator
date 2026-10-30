use orch_protocol::{AgentStateView as State, CreateSession, Reply, Request, SessionView};

use crate::common::*;

fn agent_is(state: State) -> impl Fn(&SessionView) -> bool {
    move |view| view.agent_state == Some(state)
}

#[tokio::test]
async fn a_new_session_stores_the_repos_default_agent() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    let id = running_session(&env, &mut client, "Default agent").await;

    assert_eq!(client.sessions[&id].agent, "claude");
}

#[tokio::test]
async fn a_session_keeps_its_agent_after_the_repos_default_changes() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Stay on claude").await;
    let mut pane = env.pane(&id, PANE).await;
    pane.type_line("exit 0").await;
    client.until(&id, "Exited", agent_is(State::Exited)).await;

    env.write_config("[defaults.agent]\nname = \"pi\"\n");
    let resume = Request::Resume {
        session: id.clone(),
    };
    assert_eq!(client.request(resume).await, Ok(Reply::Done));

    let view = client
        .until(&id, "running again", |view| {
            matches!(view.agent_state, Some(State::Starting | State::Working))
        })
        .await;
    assert_eq!(view.agent, "claude");
    assert_eq!(view.error, None);
}

#[tokio::test]
async fn a_session_of_an_unknown_agent_is_errored_and_never_runs_as_claude() {
    let env = Env::new();
    env.write_config("[defaults.agent]\nname = \"nope\"\n");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let repo = env.repo("app");

    let id = client.create(CreateSession::new(&repo, "Unknown")).await;

    let view = client.until(&id, "Errored", agent_is(State::Errored)).await;
    assert_eq!(view.agent, "nope");
    let error = view.error.unwrap_or_default();
    assert!(error.contains("unknown Agent \"nope\""), "{error}");
    assert_eq!(view.holder_pid, None);
}

#[tokio::test]
async fn hooks_and_taps_from_another_agent_are_dropped() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Mismatch").await;
    let mut pane = env.pane(&id, PANE).await;

    pane.type_line(&format!("hook --agent pi {}", hook("SessionStart", "")))
        .await;
    pane.type_line(r#"tap --agent pi {"session_id":"conv-1","cost":{"total_cost_usd":9.0}}"#)
        .await;
    pane.type_line(r#"tap --agent claude {"session_id":"conv-1","cost":{"total_cost_usd":1.0}}"#)
        .await;

    let view = client
        .until(&id, "the Claude sample", |view| view.cost_usd.is_some())
        .await;
    assert_eq!(view.cost_usd, Some(1.0));
    assert_eq!(view.agent_state, Some(State::Starting));

    pane.type_line(&format!("hook --agent claude {}", hook("SessionStart", "")))
        .await;
    client.until(&id, "Idle", agent_is(State::Idle)).await;
}

#[tokio::test]
async fn an_adopted_holder_keeps_the_agent_it_was_started_for() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Adopt me").await;
    client
        .until(&id, "held", |view| view.holder_pid.is_some())
        .await;

    daemon.kill();
    env.lose_state_db();
    env.write_config("[defaults.agent]\nname = \"pi\"\n");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let adopted = client
        .until(&id, "adopted", |view| {
            view.flags.recovered && view.holder_pid.is_some()
        })
        .await;

    assert_eq!(adopted.agent, "claude");
    let mut pane = env.pane(&id, PANE).await;
    pane.hook(&hook("SessionStart", "")).await;
    client.until(&id, "Idle", agent_is(State::Idle)).await;
}
