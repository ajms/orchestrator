use orch_core::SessionId;
use orch_protocol::{CommitView, CreateSession, PhaseView, Reply, Request};

use crate::common::*;

fn session_processes(id: &SessionId) -> Vec<i32> {
    let marker = format!("ORCH_SESSION={}", id.as_str());
    std::fs::read_dir("/proc")
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<i32>().ok())
        .filter(|pid| {
            std::fs::read(format!("/proc/{pid}/environ")).is_ok_and(|environ| {
                environ
                    .split(|byte| *byte == 0)
                    .any(|var| var == marker.as_bytes())
            })
        })
        .filter(|pid| !process_gone(*pid))
        .collect()
}

async fn discard(client: &mut TestClient, id: &SessionId) {
    let reply = client
        .request(Request::Discard {
            session: id.clone(),
        })
        .await;
    assert_eq!(reply, Ok(Reply::Done));
    client
        .until(id, "Discarded and cleaned up", |view| {
            view.phase == PhaseView::Discarded && view.port_base.is_none()
        })
        .await;
}

#[tokio::test]
async fn discarding_shows_what_would_be_lost_then_throws_the_session_away() {
    let env = Env::new();
    let repo = env.repo("app");
    let teardown = env.path("teardown.log");
    env.write_config(&format!(
        "[repos.{repo:?}]\nteardown = \"echo torn down > {}\"\n",
        teardown.display()
    ));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut client, "Throwaway").await;
    let view = client.sessions[&id].clone();
    let holder = client.holder_pid(&id).unwrap();
    commit(&view.worktree, "kept.txt", "kept\n");
    let commit_id = git(&view.worktree, &["rev-parse", "HEAD"]);
    std::fs::write(view.worktree.join("scratch.txt"), "scratch\n").unwrap();
    std::fs::write(view.worktree.join("README.md"), "edited\n").unwrap();

    let preview = client
        .request(Request::DiscardPreview {
            session: id.clone(),
        })
        .await;
    assert_eq!(
        preview,
        Ok(Reply::DiscardPreview {
            uncommitted: vec!["README.md".into(), "scratch.txt".into()],
            unlanded: vec![CommitView {
                id: commit_id,
                subject: "add kept.txt".into(),
            }],
        })
    );

    pane.hook(&hook("UserPromptSubmit", "")).await;
    client
        .until(&id, "Working", |view| {
            view.agent == Some(orch_protocol::AgentStateView::Working)
        })
        .await;
    discard(&mut client, &id).await;

    assert!(!view.worktree.exists());
    assert_eq!(git(&repo, &["branch", "--list", &view.branch]), "");
    assert_eq!(std::fs::read_to_string(&teardown).unwrap(), "torn down\n");
    assert_eq!(client.sessions[&id].holder_pid, None);
    wait_until("the Holder to exit", || process_gone(holder)).await;
}

#[tokio::test]
async fn a_session_whose_setup_failed_can_be_discarded() {
    let env = Env::new();
    let repo = env.repo("app");
    env.write_config(&format!("[repos.{repo:?}]\nsetup = \"exit 1\"\n"));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = client.create(CreateSession::new(&repo, "Broken")).await;
    let failed = client
        .until(&id, "Setup failed", |view| {
            view.phase == PhaseView::SetupFailed
        })
        .await;

    discard(&mut client, &id).await;

    assert!(!failed.worktree.exists());
    assert_eq!(git(&repo, &["branch", "--list", &failed.branch]), "");
}

#[tokio::test]
async fn discarding_while_setting_up_stops_the_setup_script_and_never_starts_the_agent() {
    let env = Env::new();
    let repo = env.repo("app");
    env.write_config(&format!(
        "[repos.{repo:?}]\nsetup = \"echo started; sleep 30\"\n"
    ));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = client.create(CreateSession::new(&repo, "Slow")).await;
    let setting_up = client
        .until(&id, "Setup running", |view| {
            view.setup_output
                .as_deref()
                .is_some_and(|output| output.contains("started"))
        })
        .await;
    assert_eq!(setting_up.phase, PhaseView::SettingUp);

    discard(&mut client, &id).await;

    assert!(!setting_up.worktree.exists());
    wait_until("the Setup script to stop", || {
        session_processes(&id).is_empty()
    })
    .await;
    settled(&mut client).await;
    let view = &client.sessions[&id];
    assert_eq!(view.phase, PhaseView::Discarded);
    assert_eq!(view.agent, None);
}

#[tokio::test]
async fn a_discarded_session_is_gone_after_a_daemon_restart() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = idle_session(&env, &mut client, "Gone").await;
    discard(&mut client, &id).await;

    daemon.kill();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    assert!(client.session_list().await.iter().all(|view| view.id != id));
}
