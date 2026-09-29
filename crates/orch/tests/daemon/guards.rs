use orch_core::SessionId;
use orch_protocol::{AgentStateView as State, GuardChoice, Reply, Request, SessionView};

use crate::common::*;

fn write_outside(view: &SessionView, path: &str) -> String {
    hook(
        "PreToolUse",
        &format!(
            r#""tool_name":"Write","tool_input":{{"file_path":"{path}","content":"x"}},"cwd":"{}""#,
            view.worktree.display()
        ),
    )
}

async fn prompted(client: &mut TestClient, id: &SessionId) -> SessionView {
    client
        .until(id, "a Guard prompt", |view| !view.guard_prompts.is_empty())
        .await
}

async fn answer(client: &mut TestClient, id: &SessionId, guard: u64, choice: GuardChoice) {
    let reply = client
        .request(Request::AnswerGuard {
            session: id.clone(),
            guard,
            choice,
        })
        .await;
    assert_eq!(reply, Ok(Reply::Done));
}

async fn settle(client: &mut TestClient, pane: &mut PaneView, id: &SessionId, marker: &str) {
    pane.type_line(&format!("print {marker}")).await;
    pane.wait_for_text(marker).await;
    pane.hook(&hook("Stop", "")).await;
    client
        .until(id, "Idle", |view| view.agent == Some(State::Idle))
        .await;
}

#[tokio::test]
async fn a_guard_hit_asks_the_user_and_relays_a_denial_to_the_agent() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Guarded").await;
    let view = client.sessions[&id].clone();
    let mut pane = env.pane(&id, PANE).await;

    pane.hook(&write_outside(&view, "/etc/hosts")).await;
    let asking = prompted(&mut client, &id).await;
    assert_eq!(asking.agent, Some(State::NeedsInput));
    let prompt = &asking.guard_prompts[0];
    assert_eq!(prompt.tool, "Write");
    assert_eq!(
        prompt.kind,
        orch_protocol::GuardKindView::WriteOutsideWorktree
    );
    assert_eq!(prompt.target, "/etc/hosts");

    answer(&mut client, &id, prompt.id, GuardChoice::Deny).await;
    pane.wait_for_text(r#""permissionDecision":"deny""#).await;
    client
        .until(&id, "no Guard prompt", |view| view.guard_prompts.is_empty())
        .await;
}

#[tokio::test]
async fn allowing_once_asks_again_but_allowing_for_the_session_does_not() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Guarded").await;
    let view = client.sessions[&id].clone();
    let mut pane = env.pane(&id, PANE).await;

    pane.hook(&write_outside(&view, "/etc/hosts")).await;
    let prompt = prompted(&mut client, &id).await.guard_prompts[0].clone();
    answer(&mut client, &id, prompt.id, GuardChoice::AllowOnce).await;
    settle(&mut client, &mut pane, &id, "first-done").await;

    pane.hook(&write_outside(&view, "/etc/hosts")).await;
    let prompt = prompted(&mut client, &id).await.guard_prompts[0].clone();
    answer(&mut client, &id, prompt.id, GuardChoice::AllowForSession).await;
    settle(&mut client, &mut pane, &id, "second-done").await;

    client.history.clear();
    pane.hook(&write_outside(&view, "/etc/hosts")).await;
    settle(&mut client, &mut pane, &id, "third-done").await;
    assert!(
        client
            .history
            .iter()
            .all(|view| view.guard_prompts.is_empty())
    );
    assert!(!pane.text().contains("deny"));
}

#[tokio::test]
async fn guards_switched_off_let_the_agent_act_outside_its_session() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Unguarded").await;
    let view = client.sessions[&id].clone();
    let mut pane = env.pane(&id, PANE).await;

    let off = client
        .request(Request::SetGuards {
            session: id.clone(),
            enabled: false,
        })
        .await;
    assert_eq!(off, Ok(Reply::Done));
    client
        .until(&id, "Guards off", |view| !view.guards_enabled)
        .await;
    client.history.clear();
    pane.hook(&write_outside(&view, "/etc/hosts")).await;
    settle(&mut client, &mut pane, &id, "unguarded-done").await;
    assert!(
        client
            .history
            .iter()
            .all(|view| view.guard_prompts.is_empty())
    );

    client
        .request(Request::SetGuards {
            session: id.clone(),
            enabled: true,
        })
        .await
        .unwrap();
    pane.hook(&write_outside(&view, "/etc/hosts")).await;
    prompted(&mut client, &id).await;
}

#[tokio::test]
async fn guard_settings_and_session_allowances_survive_a_daemon_restart() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Remember").await;
    let view = client.sessions[&id].clone();
    let mut pane = env.pane(&id, PANE).await;
    pane.hook(&write_outside(&view, "/etc/hosts")).await;
    let prompt = prompted(&mut client, &id).await.guard_prompts[0].clone();
    assert_eq!(
        prompt.kind,
        orch_protocol::GuardKindView::WriteOutsideWorktree
    );
    answer(&mut client, &id, prompt.id, GuardChoice::AllowForSession).await;
    settle(&mut client, &mut pane, &id, "allowed").await;
    client
        .request(Request::SetGuards {
            session: id.clone(),
            enabled: false,
        })
        .await
        .unwrap();
    client
        .until(&id, "Guards off", |view| !view.guards_enabled)
        .await;

    daemon.kill();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    assert!(!client.session_list().await[0].guards_enabled);
    client
        .request(Request::SetGuards {
            session: id.clone(),
            enabled: true,
        })
        .await
        .unwrap();
    let mut pane = env.pane(&id, PANE).await;
    client.history.clear();
    pane.hook(&write_outside(&view, "/etc/hosts")).await;
    settle(&mut client, &mut pane, &id, "still-allowed").await;
    assert!(
        client
            .history
            .iter()
            .all(|view| view.guard_prompts.is_empty())
    );
    pane.hook(&write_outside(&view, "/etc/passwd")).await;
    prompted(&mut client, &id).await;
}
