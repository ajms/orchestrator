use orch_protocol::{AgentStateView as State, CreateSession, PhaseView, Reply, Request};

use crate::bwrap::bwrap_runs;
use crate::common::*;

const PI: &str = "[defaults.agent]\nname = \"pi\"\n";

fn pi_env() -> Env {
    let env = Env::new();
    let orch = env!("CARGO_BIN_EXE_orch");
    env.write_config(&format!(
        "{PI}[defaults.agents.pi]\nbinary = {orch:?}\nargs = [\"fake-agent\", \"--\"]\n"
    ));
    let user = env.path("home/.pi/agent");
    std::fs::create_dir_all(user.join("sessions")).unwrap();
    std::fs::write(user.join("settings.json"), r#"{"theme":"dark"}"#).unwrap();
    env
}

#[tokio::test]
async fn a_pi_session_runs_pi_from_its_agent_profile() {
    let env = pi_env();
    let settings = env.path("home/.pi/agent/settings.json");
    let modified = settings.metadata().unwrap().modified().unwrap();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let repo = env.repo("app");

    let id = client.create(CreateSession::new(&repo, "Fix it")).await;

    let view = client
        .until(&id, "running", |view| view.agent_state.is_some())
        .await;
    assert_eq!(view.agent, "pi");
    assert_eq!(view.agent_state, Some(State::Unknown));
    assert_eq!(view.conversation.as_deref(), Some(id.as_str()));
    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text("arg> -na").await;
    pane.wait_for_text(&format!("session-id> {}", id.as_str()))
        .await;
    pane.wait_for_text("prompt> Fix it").await;

    let profile = env.path("state/orchestrator/agent-profiles/pi");
    pane.type_line("env PI_CODING_AGENT_DIR").await;
    pane.wait_for_text(&format!("PI_CODING_AGENT_DIR={}", profile.display()))
        .await;
    pane.type_line("env PI_OFFLINE").await;
    pane.wait_for_text("PI_OFFLINE=1").await;
    let generated = std::fs::read_to_string(profile.join("settings.json")).unwrap();
    assert!(generated.contains("\"theme\": \"dark\""), "{generated}");
    assert_eq!(settings.metadata().unwrap().modified().unwrap(), modified);
}

#[tokio::test]
async fn a_pi_session_resumes_the_same_conversation_after_a_reboot() {
    let env = pi_env();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let repo = env.repo("app");
    let id = client.create(CreateSession::new(&repo, "Fix it")).await;
    client
        .until(&id, "held", |view| view.holder_pid.is_some())
        .await;
    let holder = client.holder_pid(&id).unwrap();

    daemon.kill();
    kill(holder, "-KILL");
    wait_until("the Holder to die", || process_gone(holder)).await;
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
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
        .until(&id, "running again", |view| view.agent_state.is_some())
        .await;
    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text(&format!("session-id> {}", id.as_str()))
        .await;
    pane.type_line("print announced").await;
    pane.wait_for_text("announced").await;
    assert!(!pane.text().contains("prompt>"));
}

#[tokio::test]
async fn sandboxed_pi_sees_its_profile_but_not_the_users_sessions() {
    if !bwrap_runs() {
        return;
    }
    let env = pi_env();
    env.real_bwrap();
    let own = env.path("home/.pi/agent/sessions/--home--/own.jsonl");
    std::fs::create_dir_all(own.parent().unwrap()).unwrap();
    std::fs::write(&own, "mine").unwrap();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let repo = env.repo("app");
    let id = client.create(CreateSession::new(&repo, "Boxed")).await;
    client
        .until(&id, "running", |view| view.agent_state.is_some())
        .await;
    let mut pane = env.pane(&id, PANE).await;
    let profile = env.path("state/orchestrator/agent-profiles/pi");

    for (command, path, outcome) in [
        ("exists", own.clone(), "no"),
        ("exists", env.path("home/.pi/agent/settings.json"), "yes"),
        ("write", env.path("home/.pi/agent/settings.json"), "denied"),
        ("write", profile.join("trust.json"), "ok"),
        ("write", profile.join("settings.json"), "ok"),
        ("write", profile.join("extensions/planted.ts"), "denied"),
    ] {
        let line = format!("{command} {}", path.display());
        pane.type_line(&line).await;
        let answer = format!("{line} -> ");
        pane.wait_for(&answer, |pane| pane.text().contains(&answer))
            .await;
        let text = pane.text();
        let got = text[text.find(&answer).unwrap() + answer.len()..]
            .lines()
            .next()
            .unwrap()
            .trim()
            .to_string();
        match outcome {
            "denied" => assert_ne!(got, "ok", "{line}"),
            _ => assert_eq!(got, outcome, "{line}"),
        }
    }
}
