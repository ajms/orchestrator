use orch_agent::GuardAnswer;
use orch_core::SessionId;
use orch_holder::{
    AgentStatus, FromHolder, Hello, HolderEvent, PROTOCOL_VERSION as HOLDER_PROTOCOL, ToHolder,
    read_frame_async, write_frame_async,
};
use orch_protocol::{AgentStateView as State, CreateSession, Reply, Request, SessionView};

use crate::common::*;

const PRE_TOOL_USE: &str = r#"{"hook_event_name":"PreToolUse","session_id":"conv-1","tool_name":"Bash","tool_input":{"command":"git push --force"},"cwd":"/"}"#;

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

#[tokio::test]
async fn a_missing_binary_for_a_sessions_agent_leaves_it_errored_with_the_reason() {
    let env = Env::new();
    env.write_config(UNSANDBOXED);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Lost binary").await;
    let mut pane = env.pane(&id, PANE).await;
    pane.type_line("exit 0").await;
    client.until(&id, "Exited", agent_is(State::Exited)).await;

    env.write_config_with_agent(
        &format!("{UNSANDBOXED}\n[defaults.agent]\nname = \"pi\"\n"),
        "/nonexistent/claude",
    );
    let resume = Request::Resume {
        session: id.clone(),
    };
    assert_eq!(client.request(resume).await, Ok(Reply::Done));

    let view = client
        .until(&id, "a failed launch", |view| view.error.is_some())
        .await;
    assert_eq!(view.agent_state, Some(State::Errored));
    assert_eq!(view.agent, "claude");
    let error = view.error.unwrap_or_default();
    assert!(error.contains("/nonexistent/claude"), "{error}");
}

async fn hold_asking_a_guard_for(
    socket: std::path::PathBuf,
    session: SessionId,
    holder_pid: u32,
    agent: &'static str,
) -> tokio::sync::mpsc::Receiver<GuardAnswer> {
    let (answers, answered) = tokio::sync::mpsc::channel(4);
    let _ = std::fs::remove_file(&socket);
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let (session, answers) = (session.clone(), answers.clone());
            tokio::spawn(async move {
                let (mut reader, mut writer) = stream.into_split();
                while let Ok(Some(message)) = read_frame_async::<ToHolder>(&mut reader).await {
                    match message {
                        ToHolder::Attach { .. } => {
                            let hello = Hello {
                                version: HOLDER_PROTOCOL,
                                session: session.clone(),
                                cwd: None,
                                base: None,
                                port_block: None,
                                holder_pid,
                                agent_pid: None,
                                agent_status: AgentStatus::Running,
                                agent: "claude".into(),
                                sandboxed: false,
                            };
                            let _ = write_frame_async(&mut writer, &FromHolder::Hello(hello)).await;
                            let event = HolderEvent::Hook {
                                payload: PRE_TOOL_USE.into(),
                                guard: Some(7),
                                agent: agent.into(),
                            };
                            let event = FromHolder::Event { seq: 1000, event };
                            let _ = write_frame_async(&mut writer, &event).await;
                        }
                        ToHolder::GuardAnswer { id: 7, answer } => {
                            let _ = answers.send(answer).await;
                        }
                        _ => {}
                    }
                }
            });
        }
    });
    answered
}

#[tokio::test]
async fn a_guard_from_another_agent_is_answered_ask() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Guarded").await;
    let holder = client
        .until(&id, "held", |view| view.holder_pid.is_some())
        .await
        .holder_pid
        .unwrap() as i32;
    daemon.kill();
    kill(holder, "-KILL");
    wait_until("the Holder is gone", || process_gone(holder)).await;

    let mut stand_in = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    let mut answered =
        hold_asking_a_guard_for(env.holder_socket(&id), id.clone(), stand_in.id(), "pi").await;
    let _daemon = env.start_daemon().await;

    let answer = tokio::time::timeout(std::time::Duration::from_secs(5), answered.recv()).await;
    let _ = stand_in.kill();
    let _ = stand_in.wait();
    assert_eq!(answer.unwrap(), Some(GuardAnswer::Ask));
}
