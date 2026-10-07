use orch_core::SessionId;
use orch_protocol::{Landing, LandingMode, PhaseView, Reply, Request, RequestError};

use crate::common::*;

async fn land(
    client: &mut TestClient,
    id: &SessionId,
    message: &str,
) -> Result<Reply, RequestError> {
    client
        .request(Request::Land {
            session: id.clone(),
            landing: Landing::Squash {
                message: message.into(),
            },
            skip_teardown: false,
        })
        .await
}

#[tokio::test]
async fn a_squash_landing_moves_the_whole_worktree_onto_the_base_and_cleans_up() {
    let env = Env::new();
    let repo = env.repo("app");
    let teardown = env.path("teardown.log");
    env.write_config(&format!(
        "[repos.{repo:?}]\nteardown = \"echo torn down $ORCH_PORT_BASE > {}\"\n",
        teardown.display()
    ));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = idle_session(&env, &mut client, "Add a feature").await;
    let view = client.sessions[&id].clone();
    let holder = client.holder_pid(&id).unwrap();
    commit(&view.worktree, "committed.txt", "committed\n");
    std::fs::write(view.worktree.join("untracked.txt"), "untracked\n").unwrap();

    let landed = land(&mut client, &id, "Add a feature\n\nWith details").await;
    let Ok(Reply::Landed {
        commit,
        warning: None,
    }) = landed
    else {
        panic!("Landing failed: {landed:?}");
    };

    assert_eq!(git(&repo, &["rev-parse", "main"]), commit);
    assert_eq!(
        git(&repo, &["log", "-1", "--format=%B", "main"]),
        "Add a feature\n\nWith details"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("committed.txt")).unwrap(),
        "committed\n"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("untracked.txt")).unwrap(),
        "untracked\n"
    );
    assert!(!view.worktree.exists());
    assert_eq!(git(&repo, &["branch", "--list", &view.branch]), "");
    assert_eq!(
        std::fs::read_to_string(&teardown).unwrap(),
        "torn down 20000\n"
    );
    let done = client
        .until(&id, "Landed and cleaned up", |view| {
            view.phase == PhaseView::Landed && view.port_base.is_none()
        })
        .await;
    assert_eq!(done.holder_pid, None);
    wait_until("the Holder to exit", || process_gone(holder)).await;
}

fn refusal(reply: Result<Reply, RequestError>) -> String {
    match reply {
        Err(RequestError::Refused { message }) => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn a_squash_landing_is_refused_while_the_main_checkout_of_the_base_is_dirty() {
    let env = Env::new();
    let repo = env.repo("app");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = idle_session(&env, &mut client, "Change the readme").await;
    let view = client.sessions[&id].clone();
    let before = git(&repo, &["rev-parse", "main"]);
    std::fs::write(view.worktree.join("README.md"), "agent\n").unwrap();
    std::fs::write(repo.join("README.md"), "local edit\n").unwrap();

    let message = refusal(land(&mut client, &id, "Change the readme").await);

    assert!(message.contains("local changes"), "{message}");
    assert_eq!(git(&repo, &["rev-parse", "main"]), before);
    assert_eq!(
        std::fs::read_to_string(repo.join("README.md")).unwrap(),
        "local edit\n"
    );
    assert!(view.worktree.join("README.md").exists());
    settled(&mut client).await;
    assert_eq!(client.sessions[&id].phase, PhaseView::Active);
}

#[tokio::test]
async fn landing_is_refused_while_the_agent_is_working_or_needs_input() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut client, "Busy").await;
    let view = client.sessions[&id].clone();
    std::fs::write(view.worktree.join("new.txt"), "x\n").unwrap();

    pane.hook(&hook("UserPromptSubmit", "")).await;
    client
        .until(&id, "Working", |view| {
            view.agent == Some(orch_protocol::AgentStateView::Working)
        })
        .await;
    assert!(refusal(land(&mut client, &id, "Busy").await).contains("Working"));

    pane.hook(&hook("PermissionRequest", r#""tool_name":"Bash""#))
        .await;
    client
        .until(&id, "Needs input", |view| {
            view.agent == Some(orch_protocol::AgentStateView::NeedsInput)
        })
        .await;
    assert!(refusal(land(&mut client, &id, "Busy").await).contains("Needs input"));
    assert!(view.worktree.join("new.txt").exists());
}

#[tokio::test]
async fn landing_a_worktree_without_changes_is_refused() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = idle_session(&env, &mut client, "Nothing").await;

    let message = refusal(land(&mut client, &id, "Nothing").await);

    assert!(message.contains("no changes"), "{message}");
}

#[tokio::test]
async fn a_conflicting_landing_changes_nothing_and_hands_the_rebase_to_the_agent() {
    let env = Env::new();
    let repo = env.repo("app");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut client, "Reword").await;
    let view = client.sessions[&id].clone();
    commit(&view.worktree, "README.md", "agent\n");
    commit(&repo, "README.md", "upstream\n");
    let before = git(&repo, &["rev-parse", "main"]);

    let conflict = land(&mut client, &id, "Reword").await;

    assert_eq!(
        conflict,
        Err(RequestError::Conflict {
            paths: vec!["README.md".into()]
        })
    );
    assert_eq!(git(&repo, &["rev-parse", "main"]), before);
    assert_eq!(
        std::fs::read_to_string(repo.join("README.md")).unwrap(),
        "upstream\n"
    );
    assert!(view.worktree.exists());
    let flagged = client
        .until(&id, "needs rebase", |view| view.flags.needs_rebase)
        .await;
    assert_eq!(flagged.phase, PhaseView::Active);
    pane.wait_for_text("git rebase main").await;

    git(&view.worktree, &["reset", "-q", "--hard", "main"]);
    commit(&view.worktree, "README.md", "upstream and agent\n");
    pane.hook(&hook("Stop", "")).await;
    client
        .until(&id, "rebased", |view| !view.flags.needs_rebase)
        .await;
    assert!(matches!(
        land(&mut client, &id, "Reword").await,
        Ok(Reply::Landed { .. })
    ));
}

async fn draft(client: &mut TestClient, id: &SessionId, mode: LandingMode) -> (String, String) {
    let reply = client
        .request(Request::Draft {
            session: id.clone(),
            mode,
        })
        .await;
    match reply {
        Ok(Reply::Drafted { title, body }) => (title, body),
        other => panic!("no draft: {other:?}"),
    }
}

#[tokio::test]
async fn the_agent_drafts_messages_in_a_forked_side_conversation() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut client, "Draft me").await;
    pane.type_line("print before-draft").await;
    pane.wait_for_text("before-draft").await;
    std::fs::write(
        client.sessions[&id].worktree.join("untracked.txt"),
        "untracked line\n",
    )
    .unwrap();
    client.history.clear();

    let (title, body) = draft(&mut client, &id, LandingMode::Squash).await;
    assert_eq!(title, "Drafted from conv-1");
    assert!(body.contains("-p --resume conv-1 --fork-session"), "{body}");
    assert!(body.contains("commit message"), "{body}");
    assert!(!body.contains("untracked line"), "{body}");

    let (_, body) = draft(&mut client, &id, LandingMode::Pr).await;
    assert!(body.contains("pull request"), "{body}");
    settled(&mut client).await;
    assert!(
        client
            .history
            .iter()
            .all(|view| view.agent == Some(orch_protocol::AgentStateView::Idle))
    );
    assert!(!pane.text().contains("Drafted"));
}

#[tokio::test]
async fn without_a_conversation_the_draft_comes_from_the_sessions_prompt() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(
        &env,
        &mut client,
        "Fix the login bug\n\nUsers are logged out",
    )
    .await;

    let (title, body) = draft(&mut client, &id, LandingMode::Squash).await;

    assert_eq!(title, "Fix the login bug");
    assert_eq!(body, "Users are logged out");
}

#[tokio::test]
async fn a_failing_teardown_after_landing_is_a_warning_next_to_the_landed_commit() {
    let env = Env::new();
    let repo = env.repo("app");
    env.write_config(&format!(
        "[repos.{repo:?}]\nteardown = \"echo cannot stop the database; exit 3\"\n"
    ));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = idle_session(&env, &mut client, "Warn").await;
    std::fs::write(client.sessions[&id].worktree.join("w.txt"), "w\n").unwrap();

    let landed = land(&mut client, &id, "Warn").await;

    let Ok(Reply::Landed {
        commit,
        warning: Some(warning),
    }) = landed
    else {
        panic!("expected a Landing with a warning: {landed:?}");
    };
    assert_eq!(git(&repo, &["rev-parse", "main"]), commit);
    assert!(warning.contains("Teardown"), "{warning}");
    assert!(warning.contains("cannot stop the database"), "{warning}");
    let view = client
        .until(&id, "Landed and cleaned up", |view| {
            view.phase == PhaseView::Landed && view.port_base.is_none()
        })
        .await;
    assert!(view.error.is_some_and(|error| error.contains("Teardown")));
}
