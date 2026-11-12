use std::process::Output;

use orch_core::{SessionId, TaskStatus};

use crate::common::*;

async fn plan(env: &Env, session: &SessionId, args: &[&str]) -> Output {
    let mut command = env.orch();
    command
        .arg("plan")
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

#[tokio::test]
async fn the_agent_writes_its_own_sessions_plan_and_the_tui_sees_it() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let mine = running_session(&env, &mut client, "Plan it").await;
    let other = running_session(&env, &mut client, "Leave it").await;

    let added = plan(&env, &mine, &["add", "Design", "--link", "#241"]).await;
    assert!(added.status.success(), "{}", stderr(&added));
    assert_eq!(stdout(&added).trim(), "added Task 1");
    let blocked = plan(&env, &mine, &["add", "Build", "--blocked-by", "1"]).await;
    assert!(blocked.status.success(), "{}", stderr(&blocked));
    let attached = plan(&env, &mine, &["attach", "2", "sub-1"]).await;
    assert!(attached.status.success(), "{}", stderr(&attached));
    let done = plan(&env, &mine, &["status", "1", "done"]).await;
    assert!(done.status.success(), "{}", stderr(&done));

    let view = client
        .until(&mine, "the Plan", |view| view.plan.progress() == (1, 2))
        .await;
    let build = view.plan.task(2).unwrap();
    assert_eq!(build.blocked_by, [1]);
    assert!(view.plan.is_ready(build));
    assert_eq!(view.plan.task(1).unwrap().status, TaskStatus::Done);

    let shown = plan(&env, &mine, &["show", "--json"]).await;
    let json: orch_core::Plan = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(json, view.plan);
    let theirs = plan(&env, &other, &["show"]).await;
    assert!(stdout(&theirs).contains("no Tasks"), "{}", stdout(&theirs));
}

#[tokio::test]
async fn cycles_unknown_tasks_and_bad_input_are_refused() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Plan it").await;
    assert!(plan(&env, &id, &["add", "A"]).await.status.success());
    assert!(
        plan(&env, &id, &["add", "B", "--blocked-by", "1"])
            .await
            .status
            .success()
    );

    let cycle = plan(&env, &id, &["block", "1", "--by", "2"]).await;
    assert!(!cycle.status.success());
    assert!(stderr(&cycle).contains("cycle"), "{}", stderr(&cycle));
    let unknown = plan(&env, &id, &["status", "9", "done"]).await;
    assert!(
        stderr(&unknown).contains("no Task 9"),
        "{}",
        stderr(&unknown)
    );
    let status = plan(&env, &id, &["status", "1", "finished"]).await;
    assert!(!status.status.success());
    let long = "x".repeat(orch_core::MAX_TITLE + 1);
    assert!(!plan(&env, &id, &["add", &long]).await.status.success());
}

#[tokio::test]
async fn outside_a_session_orch_plan_refuses() {
    let env = Env::new();
    let mut command = env.orch();
    let output = command.args(["plan", "show"]).output().unwrap();
    assert!(!output.status.success());
    assert!(stderr(&output).contains("inside an orch Session"));
}
