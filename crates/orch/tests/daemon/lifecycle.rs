use orch_protocol::{CreateSession, PhaseView};

use crate::common::*;

#[tokio::test]
async fn a_new_session_runs_its_setup_script_then_starts_the_agent_with_its_prompt() {
    let env = Env::new();
    let repo = env.repo("app");
    env.write_config(&format!(
        "[repos.{repo:?}]\nsetup = \"echo preparing $ORCH_WORKTREE; echo $ORCH_PORT_BASE > port.txt\"\n"
    ));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    let id = client
        .create(CreateSession::new(&repo, "Fix the login bug"))
        .await;
    let setting_up = client
        .until(&id, "listed", |view| view.phase == PhaseView::SettingUp)
        .await;
    assert_eq!(setting_up.branch, "orch/fix-the-login-bug");
    assert_eq!(setting_up.base, "main");
    assert_eq!(
        setting_up.worktree,
        repo.join(".orchestrator/worktrees/fix-the-login-bug")
    );
    assert_eq!(setting_up.port_base, Some(20000));

    let active = client
        .until(&id, "Active", |view| view.phase == PhaseView::Active)
        .await;
    let worktree = active.worktree.display().to_string();
    assert!(
        active
            .setup_output
            .as_deref()
            .is_some_and(|output| output.contains(&format!("preparing {worktree}"))),
        "{:?}",
        active.setup_output
    );
    assert_eq!(
        std::fs::read_to_string(active.worktree.join("port.txt")).unwrap(),
        "20000\n"
    );

    client
        .until(&id, "running", |view| view.agent.is_some())
        .await;
    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text("prompt> Fix the login bug").await;
    pane.wait_for_text(&format!("session-id> {}", id.as_str()))
        .await;
    pane.type_line("env ORCH_PORT_BASE").await;
    pane.wait_for_text("ORCH_PORT_BASE=20000").await;
    pane.type_line("env ORCH_WORKTREE").await;
    pane.wait_for_text(&format!("ORCH_WORKTREE={worktree}"))
        .await;
}

#[tokio::test]
async fn parallel_sessions_get_distinct_port_blocks_and_branches() {
    let env = Env::new();
    let repo = env.repo("app");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let first = client.create(CreateSession::new(&repo, "Same task")).await;
    let second = client.create(CreateSession::new(&repo, "Same task")).await;
    let first = client
        .until(&first, "Active", |view| view.phase == PhaseView::Active)
        .await;
    let second = client
        .until(&second, "Active", |view| view.phase == PhaseView::Active)
        .await;
    assert_eq!(first.port_base, Some(20000));
    assert_eq!(second.port_base, Some(20010));
    assert_eq!(first.branch, "orch/same-task");
    assert_eq!(second.branch, "orch/same-task-2");
}

#[tokio::test]
async fn a_failed_creation_leaves_no_worktree_branch_or_session_behind() {
    let env = Env::new();
    let repo = env.repo("app");
    env.write_config("[ports]\nstart = 20000\nend = 20009\nblock_size = 10\n");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let first = client.create(CreateSession::new(&repo, "First task")).await;
    let refused = client
        .request(orch_protocol::Request::CreateSession(CreateSession::new(
            &repo,
            "Second task",
        )))
        .await;
    assert!(
        matches!(refused, Err(orch_protocol::RequestError::Refused { .. })),
        "{refused:?}"
    );

    assert!(!repo.join(".orchestrator/worktrees/second-task").exists());
    assert_eq!(git(&repo, &["branch", "--list", "orch/second-task"]), "");
    let mut fresh = env.client().await;
    let listed = fresh.session_list().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, first);
}
