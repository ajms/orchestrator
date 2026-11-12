use std::process::Output;

use orch_core::{Attachment, ChildReport, ChildState, SessionId};
use orch_protocol::{
    AgentStateView, CreateSession, GuardChoice, GuardKindView, Landing, Reply, Request,
    RequestError,
};

use crate::common::*;

async fn orch_session(env: &Env, session: &SessionId, args: &[&str]) -> Output {
    let mut command = env.orch();
    command
        .arg("session")
        .args(args)
        .env("ORCH_HOLDER_SOCKET", env.holder_socket(session));
    tokio::task::spawn_blocking(move || command.output().unwrap())
        .await
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

async fn spawn(env: &Env, client: &mut TestClient, parent: &SessionId, prompt: &str) -> SessionId {
    let output = orch_session(env, parent, &["spawn", "--prompt", prompt]).await;
    assert!(output.status.success(), "{}", stderr(&output));
    let child = SessionId::parse(stdout(&output).trim()).unwrap();
    client
        .until(&child, "running", |view| view.agent_state.is_some())
        .await;
    child
}

async fn make_idle(env: &Env, client: &mut TestClient, id: &SessionId) -> PaneView {
    let mut pane = env.pane(id, PANE).await;
    pane.hook(&hook("SessionStart", r#""source":"startup""#))
        .await;
    client
        .until(id, "Idle", |view| {
            view.agent_state == Some(AgentStateView::Idle)
        })
        .await;
    pane
}

async fn status(env: &Env, session: &SessionId, ids: &[&str]) -> Vec<ChildReport> {
    let mut args = vec!["status", "--json"];
    args.extend(ids);
    let output = orch_session(env, session, &args).await;
    assert!(output.status.success(), "{}", stderr(&output));
    serde_json::from_slice(&output.stdout).unwrap()
}

#[tokio::test]
async fn a_child_starts_on_its_parents_branch_and_is_attached_to_its_task() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let parent = running_session(&env, &mut client, "Lead the work").await;
    let mut add = env.orch();
    add.args(["plan", "add", "Split"])
        .env("ORCH_HOLDER_SOCKET", env.holder_socket(&parent));
    assert!(add.output().unwrap().status.success());

    let output = orch_session(
        &env,
        &parent,
        &["spawn", "--prompt", "Do one part", "--task", "1"],
    )
    .await;
    assert!(output.status.success(), "{}", stderr(&output));
    let child = SessionId::parse(stdout(&output).trim()).unwrap();

    let parent_view = client
        .until(&parent, "the child attached", |view| {
            view.plan
                .task(1)
                .is_some_and(|task| !task.attachments.is_empty())
        })
        .await;
    assert_eq!(
        parent_view.plan.task(1).unwrap().attachments,
        [Attachment::ChildSession(child.as_str().into())]
    );
    let child_view = client
        .until(&child, "running", |view| view.agent_state.is_some())
        .await;
    assert_eq!(child_view.parent.as_ref(), Some(&parent));
    assert_eq!(child_view.base, parent_view.branch);
    assert_eq!(child_view.preset, parent_view.preset);
    assert_eq!(child_view.agent, parent_view.agent);
    assert_ne!(child_view.worktree, parent_view.worktree);

    let reports = status(&env, &parent, &[]).await;
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].id, child.as_str());
    assert_eq!(reports[0].state, ChildState::Working);

    let mut pane = env.pane(&child, PANE).await;
    pane.wait_for_long_line(&format!(
        "prompt> You are a Child Session of lead-the-work (id {}), working on its Branch {}",
        parent.as_str(),
        parent_view.branch
    ))
    .await;
    pane.wait_for_long_line("You are working on its Task 1: Split.")
        .await;
    pane.wait_for_long_line("orch session done").await;
    pane.wait_for_long_line("---").await;
    pane.wait_for_long_line("Do one part").await;
    pane.type_line("env ORCH_PARENT_SESSION").await;
    pane.wait_for_text(&format!("ORCH_PARENT_SESSION={}", parent.as_str()))
        .await;
}

#[tokio::test]
async fn a_session_can_only_reach_itself_and_its_own_descendants() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let parent = running_session(&env, &mut client, "Lead").await;
    let stranger = running_session(&env, &mut client, "Other work").await;
    let child = spawn(&env, &mut client, &parent, "Part").await;

    for args in [
        vec!["status", child.as_str()],
        vec!["wait", child.as_str()],
        vec!["land", child.as_str()],
        vec!["discard", child.as_str()],
    ] {
        let output = orch_session(&env, &stranger, &args).await;
        assert!(!output.status.success(), "{args:?}");
        assert!(
            stderr(&output).contains("not"),
            "{args:?}: {}",
            stderr(&output)
        );
    }
    let upward = orch_session(&env, &child, &["discard", parent.as_str()]).await;
    assert!(!upward.status.success());
    let itself = orch_session(&env, &child, &["land", child.as_str()]).await;
    assert!(!itself.status.success());
    let garbage = orch_session(&env, &parent, &["status", "../../etc"]).await;
    assert!(!garbage.status.success());
    let top_done = orch_session(&env, &parent, &["done", "finished"]).await;
    assert!(
        stderr(&top_done).contains("no parent"),
        "{}",
        stderr(&top_done)
    );
    assert!(client.sessions.contains_key(&child));
}

#[tokio::test]
async fn children_nest_only_as_deep_as_the_limit() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let top = running_session(&env, &mut client, "Top").await;
    let child = spawn(&env, &mut client, &top, "Child").await;
    let grandchild = spawn(&env, &mut client, &child, "Grandchild").await;

    let too_deep = orch_session(&env, &grandchild, &["spawn", "--prompt", "Deeper"]).await;
    assert!(!too_deep.status.success());
    assert!(
        stderr(&too_deep).contains("at most 2 levels"),
        "{}",
        stderr(&too_deep)
    );
    let reports = status(&env, &top, &[]).await;
    assert_eq!(reports.len(), 2);
}

#[tokio::test]
async fn a_looser_preset_for_a_child_needs_the_users_approval() {
    let env = Env::new();
    let repo = env.repo("app");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let create = CreateSession {
        preset: Some("ask".into()),
        ..CreateSession::new(&repo, "Careful work")
    };
    let parent = running_on(&mut client, create).await;

    let asking = {
        let env_socket = env.holder_socket(&parent);
        let mut command = env.orch();
        command
            .args([
                "session", "spawn", "--prompt", "Go fast", "--preset", "auto",
            ])
            .env("ORCH_HOLDER_SOCKET", env_socket);
        tokio::task::spawn_blocking(move || command.output().unwrap())
    };
    let view = client
        .until(&parent, "a Preset prompt", |view| {
            !view.guard_prompts.is_empty()
        })
        .await;
    let prompt = &view.guard_prompts[0];
    assert_eq!(prompt.kind, GuardKindView::ChildPreset);
    assert_eq!(prompt.target, "auto");
    let reply = client
        .request(Request::AnswerGuard {
            session: parent.clone(),
            guard: prompt.id,
            choice: GuardChoice::Deny,
        })
        .await;
    assert_eq!(reply, Ok(Reply::Done));
    let output = asking.await.unwrap();
    assert!(!output.status.success());
    assert!(stderr(&output).contains("looser"), "{}", stderr(&output));
    assert!(status(&env, &parent, &[]).await.is_empty());

    let stricter = orch_session(
        &env,
        &parent,
        &["spawn", "--prompt", "Plan it", "--preset", "plan"],
    )
    .await;
    assert!(stricter.status.success(), "{}", stderr(&stricter));
}

#[tokio::test]
async fn done_hands_the_summary_to_a_waiting_parent() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let parent = running_session(&env, &mut client, "Lead").await;
    let child = spawn(&env, &mut client, &parent, "Part").await;
    let _pane = make_idle(&env, &mut client, &child).await;

    let waiting = {
        let mut command = env.orch();
        command
            .args(["session", "wait", child.as_str()])
            .env("ORCH_HOLDER_SOCKET", env.holder_socket(&parent));
        tokio::task::spawn_blocking(move || command.output().unwrap())
    };
    let done = orch_session(&env, &child, &["done", "Built the parser"]).await;
    assert!(done.status.success(), "{}", stderr(&done));

    let waited = tokio::time::timeout(WAIT, waiting).await.unwrap().unwrap();
    assert!(waited.status.success(), "{}", stderr(&waited));
    assert!(stdout(&waited).contains("[done]"), "{}", stdout(&waited));
    assert!(stdout(&waited).contains("Built the parser"));
    let view = client
        .until(&child, "Done", |view| view.done_summary.is_some())
        .await;
    assert_eq!(view.done_summary.as_deref(), Some("Built the parser"));
    let reports = status(&env, &parent, &[child.as_str()]).await;
    assert_eq!(reports[0].state, ChildState::Done);
    assert_eq!(reports[0].summary.as_deref(), Some("Built the parser"));
}

#[tokio::test]
async fn done_hands_the_summary_to_a_parent_waiting_on_another_child() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (parent, mut parent_pane) = idle_session(&env, &mut client, "Lead").await;
    let slow = spawn(&env, &mut client, &parent, "Slow").await;
    let quick = spawn(&env, &mut client, &parent, "Quick").await;
    let _slow_pane = make_idle(&env, &mut client, &slow).await;
    let _quick_pane = make_idle(&env, &mut client, &quick).await;

    let waiting = {
        let mut command = env.orch();
        command
            .args(["session", "wait", slow.as_str()])
            .env("ORCH_HOLDER_SOCKET", env.holder_socket(&parent));
        tokio::task::spawn_blocking(move || command.output().unwrap())
    };
    let done = orch_session(&env, &quick, &["done", "Quick part built"]).await;
    assert!(done.status.success(), "{}", stderr(&done));
    parent_pane.wait_for_text("Quick part built").await;

    let done = orch_session(&env, &slow, &["done", "Slow part built"]).await;
    assert!(done.status.success(), "{}", stderr(&done));
    let waited = tokio::time::timeout(WAIT, waiting).await.unwrap().unwrap();
    assert!(stdout(&waited).contains("Slow part built"));
}

#[tokio::test]
async fn a_parent_cannot_land_a_child_that_isnt_done_but_the_user_can() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (parent, _parent_pane) = idle_session(&env, &mut client, "Lead").await;
    let child = spawn(&env, &mut client, &parent, "Part").await;
    let _pane = make_idle(&env, &mut client, &child).await;
    let child_view = client.sessions[&child].clone();
    std::fs::write(child_view.worktree.join("part.txt"), "half\n").unwrap();

    let landed = orch_session(&env, &parent, &["land", child.as_str()]).await;
    assert!(!landed.status.success());
    assert!(
        stderr(&landed).contains("isn't Done"),
        "{}",
        stderr(&landed)
    );
    assert!(client.sessions[&child].land_failed.is_none());

    client
        .request(Request::Land {
            session: child.clone(),
            landing: Landing::Squash {
                message: "Part".into(),
            },
            skip_teardown: false,
        })
        .await
        .unwrap();
    client.until_removed(&child).await;
}

#[tokio::test]
async fn landing_a_child_squashes_it_onto_the_parent_and_keeps_the_parents_work() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (parent, _parent_pane) = idle_session(&env, &mut client, "Lead").await;
    let child = spawn(&env, &mut client, &parent, "Part").await;
    let _pane = make_idle(&env, &mut client, &child).await;
    let parent_view = client.sessions[&parent].clone();
    let child_view = client.sessions[&child].clone();
    std::fs::write(child_view.worktree.join("part.txt"), "part\n").unwrap();
    std::fs::write(parent_view.worktree.join("draft.txt"), "draft\n").unwrap();
    assert!(
        orch_session(&env, &child, &["done", "Wrote the part"])
            .await
            .status
            .success()
    );

    let refused = client
        .request(Request::Land {
            session: parent.clone(),
            landing: Landing::Squash {
                message: "All".into(),
            },
            skip_teardown: false,
        })
        .await;
    let Err(RequestError::Refused { message }) = refused else {
        panic!("the parent Landed with an open child: {refused:?}");
    };
    assert!(message.contains("open Child Sessions"), "{message}");

    let landed = orch_session(&env, &parent, &["land", child.as_str()]).await;
    assert!(landed.status.success(), "{}", stderr(&landed));
    client.until_removed(&child).await;
    assert_eq!(
        std::fs::read_to_string(parent_view.worktree.join("part.txt")).unwrap(),
        "part\n"
    );
    assert_eq!(
        std::fs::read_to_string(parent_view.worktree.join("draft.txt")).unwrap(),
        "draft\n"
    );
    let subject = git(&parent_view.worktree, &["log", "-1", "--format=%B"]);
    assert!(subject.contains("Wrote the part"), "{subject}");
    let status = git(&parent_view.worktree, &["status", "--porcelain"]);
    assert_eq!(status, "?? draft.txt");
}

#[tokio::test]
async fn a_conflicting_land_leaves_the_child_open_as_land_failed() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (parent, _parent_pane) = idle_session(&env, &mut client, "Lead").await;
    let child = spawn(&env, &mut client, &parent, "Part").await;
    let _pane = make_idle(&env, &mut client, &child).await;
    let parent_view = client.sessions[&parent].clone();
    let child_view = client.sessions[&child].clone();
    std::fs::write(child_view.worktree.join("shared.txt"), "child\n").unwrap();
    std::fs::write(parent_view.worktree.join("shared.txt"), "parent\n").unwrap();
    let parent_head = git(&parent_view.worktree, &["rev-parse", "HEAD"]);
    assert!(
        orch_session(&env, &child, &["done", "Wrote shared"])
            .await
            .status
            .success()
    );

    let landed = orch_session(&env, &parent, &["land", child.as_str()]).await;
    assert!(!landed.status.success());
    assert!(
        stderr(&landed).contains("shared.txt"),
        "{}",
        stderr(&landed)
    );

    let view = client
        .until(&child, "Land failed", |view| view.land_failed.is_some())
        .await;
    assert!(view.land_failed.unwrap().contains("shared.txt"));
    assert_eq!(
        git(&parent_view.worktree, &["rev-parse", "HEAD"]),
        parent_head
    );
    assert_eq!(
        std::fs::read_to_string(parent_view.worktree.join("shared.txt")).unwrap(),
        "parent\n"
    );
    let reports = status(&env, &parent, &[child.as_str()]).await;
    assert_eq!(reports[0].state, ChildState::LandFailed);
}

#[tokio::test]
async fn a_child_with_nothing_to_land_is_refused_without_marking_it_land_failed() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (parent, _parent_pane) = idle_session(&env, &mut client, "Lead").await;
    let child = spawn(&env, &mut client, &parent, "Part").await;
    let _pane = make_idle(&env, &mut client, &child).await;
    assert!(
        orch_session(&env, &child, &["done", "Nothing"])
            .await
            .status
            .success()
    );

    let landed = orch_session(&env, &parent, &["land", child.as_str()]).await;
    assert!(!landed.status.success());

    let reports = status(&env, &parent, &[child.as_str()]).await;
    assert_ne!(reports[0].state, ChildState::LandFailed);
    assert!(client.sessions[&child].land_failed.is_none());
}

#[tokio::test]
async fn discarding_a_parent_discards_its_open_children_after_listing_them() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let parent = running_session(&env, &mut client, "Lead").await;
    let child = spawn(&env, &mut client, &parent, "Part one").await;
    let grandchild = spawn(&env, &mut client, &child, "Part two").await;

    let preview = client
        .request(Request::DiscardPreview {
            session: parent.clone(),
        })
        .await;
    let Ok(Reply::DiscardPreview { children, .. }) = preview else {
        panic!("no preview: {preview:?}");
    };
    assert_eq!(children.len(), 2);

    let discarded = client
        .request(Request::Discard {
            session: parent.clone(),
            skip_teardown: false,
        })
        .await;
    assert_eq!(discarded, Ok(Reply::Done));
    for id in [&grandchild, &child, &parent] {
        client.until_removed(id).await;
    }
}

#[tokio::test]
async fn muting_a_parent_mutes_its_children() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let parent = running_session(&env, &mut client, "Lead").await;
    let child = spawn(&env, &mut client, &parent, "Part").await;
    let muted = client
        .request(Request::SetMuted {
            session: parent.clone(),
            muted: true,
        })
        .await;
    assert_eq!(muted, Ok(Reply::Done));
    client.until(&child, "muted", |view| view.flags.muted).await;
}

#[tokio::test]
async fn after_a_restart_no_child_is_left_pointing_at_an_ended_parent() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let parent = running_session(&env, &mut client, "Lead").await;
    let child = spawn(&env, &mut client, &parent, "Part").await;
    daemon.kill();
    {
        // A crash in the middle of ending the parent: it is gone, its child isn't.
        let db = env.path("state/orchestrator/state.db");
        let mut store = orch_store::Store::open(&db).unwrap();
        let mut record = store.session(&parent).unwrap().unwrap();
        record.phase = orch_core::Phase::Discarded;
        store.save_session(&record).unwrap();
    }

    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let view = client
        .until(&child, "a top-level Session", |view| view.parent.is_none())
        .await;
    assert!(view.error.unwrap().contains("Parent Session ended"));
    assert!(!client.sessions.contains_key(&parent));
}
