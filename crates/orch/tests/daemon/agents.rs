use orch_protocol::{AgentStateView as State, CreateSession, Reply, Request, RequestError};

use crate::common::*;

#[tokio::test]
async fn a_session_keeps_its_agent_when_the_repo_default_changes() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = idle_session(&env, &mut client, "Steady").await;
    let old_holder = client.holder_pid(&id).unwrap();
    let repo = env.path("repos/app");
    env.write_config(&format!("[repos.{repo:?}]\nagent = \"antigravity\"\n"));

    let restarted = client
        .request(Request::SetPreset {
            session: id.clone(),
            preset: "plan".into(),
        })
        .await;
    assert_eq!(restarted, Ok(Reply::Done));

    let after = client
        .until(&id, "restarted", |view| {
            view.holder_pid.is_some_and(|pid| pid as i32 != old_holder) && view.agent.is_some()
        })
        .await;
    assert_eq!(after.error, None);
    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text("resume> conv-1").await;
}

#[tokio::test]
async fn resuming_a_session_whose_agent_is_unknown_makes_it_errored() {
    let env = Env::new();
    let repo = env.repo("app");
    env.write_config_with_agent("", "/nonexistent/agent");
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = client.create(CreateSession::new(&repo, "Lost agent")).await;
    client
        .until(&id, "a failed launch", |view| view.error.is_some())
        .await;
    daemon.kill();
    let db = rusqlite::Connection::open(env.path("state/orchestrator/state.db")).unwrap();
    db.execute("UPDATE sessions SET agent = 'nonesuch'", [])
        .unwrap();
    drop(db);
    env.write_config("");

    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    client.session_list().await;
    let resumed = client
        .request(Request::Resume {
            session: id.clone(),
        })
        .await;
    assert_eq!(resumed, Ok(Reply::Done));

    let failed = client
        .until(&id, "Errored", |view| view.error.is_some())
        .await;
    assert_eq!(failed.agent, Some(State::Errored));
    assert_eq!(failed.holder_pid, None);
    let error = failed.error.unwrap();
    assert!(error.contains("nonesuch"), "{error}");
}

#[tokio::test]
async fn a_session_whose_agent_binary_is_missing_is_errored_naming_the_binary() {
    let env = Env::new();
    let repo = env.repo("app");
    env.write_config_with_agent("", "/nonexistent/agent");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    let id = client.create(CreateSession::new(&repo, "No binary")).await;

    let failed = client
        .until(&id, "a failed launch", |view| view.error.is_some())
        .await;
    assert_eq!(failed.agent, Some(State::Errored));
    let error = failed.error.unwrap();
    assert!(error.contains("claude"), "{error}");
    assert!(error.contains("/nonexistent/agent"), "{error}");
}

#[tokio::test]
async fn creating_a_session_for_an_unknown_agent_is_refused() {
    let env = Env::new();
    let repo = env.repo("app");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let mut create = CreateSession::new(&repo, "Other agent");
    create.agent = Some("nonesuch".into());

    let refused = client.request(Request::CreateSession(create)).await;

    assert!(
        matches!(&refused, Err(RequestError::Refused { message }) if message.contains("nonesuch")),
        "{refused:?}"
    );
    assert!(client.session_list().await.is_empty());
}
