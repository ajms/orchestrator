use orch_protocol::{AgentStateView as State, CreateSession, Landing, Reply, Request};

use crate::common::*;

#[tokio::test]
async fn landing_a_stacked_base_retargets_the_session_and_hands_it_the_rebase_once_idle() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (lower, _lower_pane) = idle_session(&env, &mut client, "Lower").await;
    let lower_view = client.sessions[&lower].clone();
    commit(&lower_view.worktree, "lower.txt", "lower\n");

    let mut create = CreateSession::new(env.path("repos/app"), "Upper");
    create.base = Some(lower_view.branch.clone());
    let upper = client.create(create).await;
    client
        .until(&upper, "running", |view| view.agent.is_some())
        .await;
    let upper_view = client.sessions[&upper].clone();
    commit(&upper_view.worktree, "upper.txt", "upper\n");
    let mut pane = env.pane(&upper, PANE).await;
    pane.hook(&hook("UserPromptSubmit", "")).await;
    client
        .until(&upper, "Working", |view| view.agent == Some(State::Working))
        .await;

    let landed = client
        .request(Request::Land {
            session: lower.clone(),
            landing: Landing::Squash {
                message: "Lower".into(),
            },
            skip_teardown: false,
        })
        .await;
    assert!(matches!(landed, Ok(Reply::Landed { .. })), "{landed:?}");

    let retargeted = client
        .until(&upper, "retargeted", |view| view.base == "main")
        .await;
    assert!(retargeted.flags.needs_rebase);
    pane.type_line("print still-working").await;
    pane.wait_for_text("still-working").await;
    assert!(!pane.text().contains("rebase"));

    pane.hook(&hook("Stop", "")).await;
    pane.wait_for_text("Landed into main").await;

    git(&upper_view.worktree, &["reset", "-q", "--hard", "main"]);
    commit(&upper_view.worktree, "upper.txt", "upper\n");
    pane.hook(&hook("Stop", "")).await;
    client
        .until(&upper, "rebased", |view| !view.flags.needs_rebase)
        .await;
}

#[tokio::test]
async fn a_retargeted_base_is_remembered_across_a_daemon_restart() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (lower, _pane) = idle_session(&env, &mut client, "Lower").await;
    let lower_view = client.sessions[&lower].clone();
    commit(&lower_view.worktree, "lower.txt", "lower\n");
    let mut create = CreateSession::new(env.path("repos/app"), "Upper");
    create.base = Some(lower_view.branch.clone());
    let upper = client.create(create).await;
    client
        .until(&upper, "running", |view| view.agent.is_some())
        .await;
    client
        .request(Request::Land {
            session: lower.clone(),
            landing: Landing::Squash {
                message: "Lower".into(),
            },
            skip_teardown: false,
        })
        .await
        .unwrap();
    client
        .until(&upper, "retargeted", |view| view.base == "main")
        .await;
    settled(&mut client).await;

    daemon.kill();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let list = client.session_list().await;
    let view = list.iter().find(|view| view.id == upper).unwrap();
    assert_eq!(view.base, "main");
    assert!(view.flags.needs_rebase);
}

#[tokio::test]
async fn a_queued_rebase_prompt_survives_a_daemon_restart() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (lower, _lower_pane) = idle_session(&env, &mut client, "Lower").await;
    let lower_view = client.sessions[&lower].clone();
    commit(&lower_view.worktree, "lower.txt", "lower\n");
    let mut create = CreateSession::new(env.path("repos/app"), "Upper");
    create.base = Some(lower_view.branch.clone());
    let upper = client.create(create).await;
    client
        .until(&upper, "running", |view| view.agent.is_some())
        .await;
    let mut pane = env.pane(&upper, PANE).await;
    pane.hook(&hook("UserPromptSubmit", "")).await;
    client
        .until(&upper, "Working", |view| view.agent == Some(State::Working))
        .await;
    client
        .request(Request::Land {
            session: lower.clone(),
            landing: Landing::Squash {
                message: "Lower".into(),
            },
            skip_teardown: false,
        })
        .await
        .unwrap();
    client
        .until(&upper, "retargeted", |view| view.base == "main")
        .await;
    settled(&mut client).await;

    daemon.kill();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    client.session_list().await;
    let mut pane = env.pane(&upper, PANE).await;
    pane.hook(&hook("Stop", "")).await;
    pane.wait_for_text("Landed into main").await;
}
