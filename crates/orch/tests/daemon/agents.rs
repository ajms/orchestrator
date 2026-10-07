use orch_core::SessionId;
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

#[tokio::test]
async fn an_unknown_agent_is_reported_before_the_preset_it_cannot_run() {
    let env = Env::new();
    let repo = env.repo("app");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let mut create = CreateSession::new(&repo, "Other agent");
    create.agent = Some("nonesuch".into());
    create.preset = Some("plan".into());

    let refused = client.request(Request::CreateSession(create)).await;

    assert!(
        matches!(&refused, Err(RequestError::Refused { message }) if message.contains("unknown Agent \"nonesuch\"")),
        "{refused:?}"
    );
}

#[tokio::test]
async fn changing_the_preset_of_a_session_whose_agent_is_unknown_names_the_agent() {
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

    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    client.session_list().await;
    let refused = client
        .request(Request::SetPreset {
            session: id.clone(),
            preset: "plan".into(),
        })
        .await;

    assert!(
        matches!(&refused, Err(RequestError::Refused { message }) if message.contains("unknown Agent \"nonesuch\"")),
        "{refused:?}"
    );
}

#[tokio::test]
async fn a_relative_agent_binary_resolves_against_the_repo_root_for_the_form_and_the_launch() {
    let env = Env::new();
    let repo = env.repo("app");
    std::fs::create_dir_all(repo.join("bin")).unwrap();
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_orch"), repo.join("bin/agent")).unwrap();
    env.write_config_with_agent("", "./bin/agent");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    let Ok(Reply::RepoSettings(settings)) = client
        .request(Request::RepoSettings { repo: repo.clone() })
        .await
    else {
        panic!("expected RepoSettings");
    };
    assert_eq!(settings.agents[0].unavailable, None);

    let id = client
        .create(CreateSession::new(&repo, "Local agent"))
        .await;
    let running = client
        .until(&id, "launched", |view| {
            view.agent.is_some() || view.error.is_some()
        })
        .await;
    assert_eq!(running.error, None);
    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text("Local agent").await;
}

#[tokio::test]
async fn a_session_recovered_from_its_live_holder_keeps_the_agent_it_runs() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Survivor").await;
    daemon.kill();
    env.lose_state_db();
    let repo = env.path("repos/app");
    env.write_config(&format!("[repos.{repo:?}]\nagent = \"antigravity\"\n"));

    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    client
        .until(&id, "adopted", |view| {
            view.flags.recovered && view.holder_pid.is_some()
        })
        .await;
    let mut pane = env.pane(&id, PANE).await;
    pane.type_line(&format!(
        "hook --agent claude {}",
        hook("SessionStart", r#""source":"startup""#)
    ))
    .await;

    client
        .until(&id, "Idle", |view| view.agent == Some(State::Idle))
        .await;
}

#[tokio::test]
async fn hook_and_tap_payloads_from_another_agent_change_nothing_and_its_own_are_applied() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Nested").await;
    let mut pane = env.pane(&id, PANE).await;
    let started = hook("SessionStart", r#""source":"startup""#);
    let context = |percent: u32| format!(r#"{{"context_window":{{"used_percentage":{percent}}}}}"#);

    pane.type_line(&format!("hook --agent antigravity {started}"))
        .await;
    pane.type_line(&format!("tap --agent claude {}", context(17)))
        .await;
    let tapped = client
        .until(&id, "Claude's statusline", |view| {
            view.context_used_percent == Some(17.0)
        })
        .await;
    assert_ne!(tapped.agent, Some(State::Idle));

    pane.type_line(&format!("tap --agent antigravity {}", context(42)))
        .await;
    pane.type_line(&format!("hook --agent claude {started}"))
        .await;
    let idle = client
        .until(&id, "Idle", |view| view.agent == Some(State::Idle))
        .await;
    assert_eq!(idle.context_used_percent, Some(17.0));
}

fn hand_held(env: &Env, name: &str, args: &[&str]) -> SessionId {
    let repo = env.path("repos/app");
    let worktree = format!(".orchestrator/worktrees/{name}");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            &format!("orch/{name}"),
            &worktree,
        ],
    );
    let id = SessionId::parse(name).unwrap();
    env.hold_with(id.as_str(), &repo.join(worktree), args);
    id
}

async fn adopted(client: &mut TestClient, id: &SessionId) {
    client.reconcile().await;
    client
        .until(id, "adopted", |view| view.holder_pid.is_some())
        .await;
}

#[tokio::test]
async fn a_holder_that_reports_no_agent_is_recovered_as_claude() {
    let env = Env::new();
    let repo = env.repo("app");
    env.write_config(&format!("[repos.{repo:?}]\nagent = \"antigravity\"\n"));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = hand_held(&env, "older-holder", &[]);

    adopted(&mut client, &id).await;
    let mut pane = env.pane(&id, PANE).await;
    pane.hook(&hook("SessionStart", r#""source":"startup""#))
        .await;

    client
        .until(&id, "Idle", |view| view.agent == Some(State::Idle))
        .await;
}

#[tokio::test]
async fn a_guard_from_another_agent_is_answered_with_ask_at_once() {
    let env = Env::new();
    env.repo("app");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = hand_held(
        &env,
        "agy-holder",
        &["--agent", "antigravity", "--guard-timeout-ms", "600000"],
    );
    adopted(&mut client, &id).await;
    let mut pane = env.pane(&id, PANE).await;

    pane.type_line(&format!(
        "hook --agent claude {}",
        hook(
            "PreToolUse",
            r#""tool_name":"Write","tool_input":{"file_path":"/etc/hosts","content":"x"}"#
        )
    ))
    .await;

    pane.wait_for_text(r#""permissionDecision":"ask""#).await;
}
