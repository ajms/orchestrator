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
    assert_eq!(view.agent_state, Some(State::Starting));
    assert_eq!(view.conversation.as_deref(), Some(id.as_str()));
    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text("arg> -na").await;
    pane.wait_for_text(&format!("session-id> {}", id.as_str()))
        .await;
    pane.wait_for_text("prompt> Fix it").await;

    let profile = env.path("state/orchestrator/agent-profiles/pi");
    let extension = profile.join("extensions/orch/extension/index.ts");
    pane.wait_for_text(&format!("arg> {}", extension.display()))
        .await;
    pane.wait_for_text(&format!("orch-bin> {}", env!("CARGO_BIN_EXE_orch")))
        .await;
    pane.wait_for_text(&format!("orch-session> {}", id.as_str()))
        .await;
    assert!(extension.is_file());
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

async fn resumed_pane(env: &Env, client: &mut TestClient, id: &orch_core::SessionId) -> PaneView {
    let resumed = client
        .request(Request::Resume {
            session: id.clone(),
        })
        .await;
    assert_eq!(resumed, Ok(Reply::Done));
    client
        .until(id, "running again", |view| {
            view.phase == PhaseView::Active && view.holder_pid.is_some()
        })
        .await;
    let mut pane = env.pane(id, PANE).await;
    pane.wait_for_text(&format!("session-id> {}", id.as_str()))
        .await;
    pane.type_line("print announced").await;
    pane.wait_for_text("announced").await;
    assert!(!pane.text().contains("prompt>"));
    pane
}

#[tokio::test]
async fn a_suspended_or_exited_pi_session_resumes_the_same_conversation() {
    let env = pi_env();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let repo = env.repo("app");
    let id = client.create(CreateSession::new(&repo, "Fix it")).await;
    client
        .until(&id, "held", |view| view.holder_pid.is_some())
        .await;

    kill(client.holder_pid(&id).unwrap(), "-KILL");
    client
        .until(&id, "Suspended", |view| view.phase == PhaseView::Suspended)
        .await;
    let mut pane = resumed_pane(&env, &mut client, &id).await;

    pane.type_line("exit 0").await;
    client
        .until(&id, "Exited", |view| {
            view.agent_state == Some(State::Exited)
        })
        .await;
    resumed_pane(&env, &mut client, &id).await;
}

#[tokio::test]
async fn an_edit_planted_in_the_profile_is_reset_at_the_next_launch() {
    let env = pi_env();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let repo = env.repo("app");
    let id = client.create(CreateSession::new(&repo, "Fix it")).await;
    client
        .until(&id, "held", |view| view.holder_pid.is_some())
        .await;
    let settings = env.path("state/orchestrator/agent-profiles/pi/settings.json");
    std::fs::write(&settings, r#"{"planted":true}"#).unwrap();
    let mut pane = env.pane(&id, PANE).await;
    pane.type_line("exit 0").await;
    client
        .until(&id, "Exited", |view| {
            view.agent_state == Some(State::Exited)
        })
        .await;

    resumed_pane(&env, &mut client, &id).await;

    let rebuilt = std::fs::read_to_string(&settings).unwrap();
    assert!(!rebuilt.contains("planted"), "{rebuilt}");
    assert!(rebuilt.contains("\"theme\": \"dark\""), "{rebuilt}");
}

fn pi_hook(payload: &str) -> String {
    format!("hook --agent pi {payload}")
}

async fn started_pi_session(
    env: &Env,
    client: &mut TestClient,
) -> (orch_core::SessionId, PaneView) {
    let repo = env.repo("app");
    let id = client.create(CreateSession::new(&repo, "Fix it")).await;
    client
        .until(&id, "Starting", |view| {
            view.agent_state == Some(State::Starting)
        })
        .await;
    let mut pane = env.pane(&id, PANE).await;
    pane.type_line(&pi_hook(&format!(
        r#"{{"event":"session_start","reason":"startup","sessionId":"{}"}}"#,
        id.as_str()
    )))
    .await;
    client
        .until(&id, "Idle", |view| view.agent_state == Some(State::Idle))
        .await;
    (id, pane)
}

async fn until_state(client: &mut TestClient, id: &orch_core::SessionId, state: State) {
    client
        .until(id, &format!("{state:?}"), |view| {
            view.agent_state == Some(state)
        })
        .await;
}

#[tokio::test]
async fn pi_hooks_drive_the_agent_state_through_a_prompt() {
    let env = pi_env();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = started_pi_session(&env, &mut client).await;

    for (payload, state) in [
        (r#"{"event":"input"}"#, State::Working),
        (
            r#"{"event":"tool_call","toolName":"question"}"#,
            State::NeedsInput,
        ),
        (
            r#"{"event":"tool_execution_end","toolName":"question","isError":false}"#,
            State::Working,
        ),
        (
            r#"{"event":"ui_prompt_start","kind":"confirm"}"#,
            State::NeedsInput,
        ),
        (
            r#"{"event":"ui_prompt_end","kind":"confirm","idle":false}"#,
            State::Working,
        ),
        (
            r#"{"event":"agent_settled","aborted":true,"stopReason":"aborted"}"#,
            State::Idle,
        ),
        (r#"{"event":"agent_start"}"#, State::Working),
        (
            r#"{"event":"agent_settled","aborted":false,"stopReason":"error","errorMessage":"429 rate limit"}"#,
            State::Errored,
        ),
    ] {
        pane.type_line(&pi_hook(payload)).await;
        until_state(&mut client, &id, state).await;
    }
}

#[tokio::test]
async fn a_new_conversation_in_pi_is_the_one_a_resume_continues() {
    let env = pi_env();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = started_pi_session(&env, &mut client).await;

    pane.type_line(&pi_hook(
        r#"{"event":"session_start","reason":"new","sessionId":"conv-2"}"#,
    ))
    .await;
    client
        .until(&id, "the new Conversation", |view| {
            view.conversation.as_deref() == Some("conv-2")
        })
        .await;
    pane.type_line("exit 0").await;
    until_state(&mut client, &id, State::Exited).await;

    let resumed = client
        .request(Request::Resume {
            session: id.clone(),
        })
        .await;
    assert_eq!(resumed, Ok(Reply::Done));
    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text("session-id> conv-2").await;
}

#[tokio::test]
async fn a_pi_session_name_is_the_title_until_it_is_cleared() {
    let env = pi_env();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = started_pi_session(&env, &mut client).await;

    pane.type_line(&pi_hook(
        r#"{"event":"session_info_changed","name":"fix login"}"#,
    ))
    .await;
    client
        .until(&id, "the title", |view| {
            view.title.as_deref() == Some("fix login")
        })
        .await;
    pane.type_line(&pi_hook(
        r#"{"event":"session_start","reason":"new","sessionId":"conv-2"}"#,
    ))
    .await;
    let renewed = client
        .until(&id, "the new Conversation", |view| {
            view.conversation.as_deref() == Some("conv-2")
        })
        .await;
    assert_eq!(renewed.title.as_deref(), Some("fix login"));

    pane.type_line(&pi_hook(r#"{"event":"session_info_changed","name":""}"#))
        .await;
    let cleared = client
        .until(&id, "the slug", |view| view.title.is_none())
        .await;
    assert_eq!(cleared.slug, "fix-it");
}

fn real_pi() -> Option<std::path::PathBuf> {
    if std::env::var("ORCH_REAL_PI").as_deref() != Ok("1") {
        return None;
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("pi"))
        .find(|pi| pi.is_file())
}

#[tokio::test]
async fn real_pi_loads_orchs_extension_reports_its_start_and_resumes() {
    let Some(pi) = real_pi() else {
        return;
    };
    let env = Env::new();
    env.write_config(&format!("{PI}[defaults.agents.pi]\nbinary = {pi:?}\n"));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let repo = env.repo("app");
    let id = client.create(CreateSession::new(&repo, "")).await;

    let started = client
        .until(&id, "pi's session_start", |view| {
            view.agent_state == Some(State::Idle)
        })
        .await;
    assert_eq!(started.conversation.as_deref(), Some(id.as_str()));
    let profile = env.path("state/orchestrator/agent-profiles/pi");
    assert!(profile.join("extensions/orch/extension/index.ts").is_file());

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
    let back = client
        .until(&id, "pi's session_start after the resume", |view| {
            view.phase == PhaseView::Active && view.agent_state == Some(State::Idle)
        })
        .await;
    assert_eq!(back.conversation.as_deref(), Some(id.as_str()));
}
