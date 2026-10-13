use std::path::PathBuf;

use orch_core::SessionId;
use orch_protocol::{CreateSession, GuardChoice, GuardKindView, Reply, Request, SessionView};

use crate::common::*;

const BOXED: &str = "[defaults.presets.boxed]\nsandbox = true\n";
const ALL_SANDBOXED: &str = "[defaults]\nsandbox = true\n\n";

struct Pushing {
    env: Env,
    remote: PathBuf,
}

fn pushing(extra: &str) -> Pushing {
    let env = Env::new();
    env.write_config(&format!("{extra}{BOXED}"));
    env.fake_bwrap();
    let repo = env.repo("app");
    let remote = env.remote(&repo);
    Pushing { env, remote }
}

async fn session(
    env: &Env,
    client: &mut TestClient,
    preset: &str,
) -> (SessionId, SessionView, PaneView) {
    let mut create = CreateSession::new(env.path("repos/app"), "Push it");
    create.preset = Some(preset.into());
    let id = client.create(create).await;
    let view = client
        .until(&id, "running", |view| view.agent.is_some())
        .await;
    let pane = env.pane(&id, PANE).await;
    (id, view, pane)
}

async fn shell(pane: &mut PaneView, script: &str) -> String {
    pane.type_line(&format!("sh {script}")).await;
    shell_result(pane, script).await
}

async fn shell_result(pane: &mut PaneView, script: &str) -> String {
    let answer = format!("sh {script} -> ");
    pane.wait_for(&answer, |pane| pane.text().contains(&answer))
        .await;
    let text = pane.text();
    let start = text.find(&answer).unwrap() + answer.len();
    text[start..].lines().next().unwrap().trim().to_string()
}

fn script(path: PathBuf, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn remote_tip(remote: &std::path::Path, branch: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(remote)
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("refs/heads/{branch}"))
        .output()
        .unwrap();
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).unwrap().trim().into())
}

#[tokio::test]
async fn a_sandboxed_agent_pushes_its_own_branch_through_the_broker() {
    let Pushing { env, remote } = pushing("");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (_, view, mut pane) = session(&env, &mut client, "boxed").await;
    commit(&view.worktree, "feature.rs", "feature\n");

    let push = format!("git push -u origin {}", view.branch);
    assert_eq!(shell(&mut pane, &push).await, "exit 0");
    assert_eq!(
        remote_tip(&remote, &view.branch),
        Some(git(&view.worktree, &["rev-parse", "HEAD"]))
    );
    pane.wait_for_text("Broker: pushed").await;
}

#[tokio::test]
async fn a_rewritten_branch_goes_through_with_a_lease_but_not_with_a_plain_force() {
    let Pushing { env, remote } = pushing("");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (_, view, mut pane) = session(&env, &mut client, "boxed").await;
    commit(&view.worktree, "feature.rs", "feature\n");
    let push = format!("git push -u origin {}", view.branch);
    assert_eq!(shell(&mut pane, &push).await, "exit 0");
    git(
        &view.worktree,
        &["commit", "-q", "--amend", "-m", "reworded"],
    );
    let rewritten = git(&view.worktree, &["rev-parse", "HEAD"]);

    assert_ne!(shell(&mut pane, "git push --force").await, "exit 0");
    pane.wait_for_text("--force-with-lease").await;
    assert_ne!(remote_tip(&remote, &view.branch), Some(rewritten.clone()));

    assert_eq!(
        shell(&mut pane, "git push --force-with-lease").await,
        "exit 0"
    );
    assert_eq!(remote_tip(&remote, &view.branch), Some(rewritten));
}

#[tokio::test]
async fn other_refs_tags_and_the_base_branch_are_refused() {
    let Pushing { env, remote } = pushing("");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (_, view, mut pane) = session(&env, &mut client, "boxed").await;
    commit(&view.worktree, "feature.rs", "feature\n");
    let main_before = remote_tip(&remote, "main");

    assert_ne!(
        shell(&mut pane, "git push origin HEAD:main").await,
        "exit 0"
    );
    pane.wait_for_text("never pushes the Base branch main")
        .await;
    assert_ne!(
        shell(&mut pane, "git push origin HEAD:refs/heads/other").await,
        "exit 0"
    );
    git(&view.worktree, &["tag", "v1"]);
    assert_ne!(shell(&mut pane, "git push origin v1").await, "exit 0");
    assert_ne!(shell(&mut pane, "git push --all origin").await, "exit 0");

    assert_eq!(remote_tip(&remote, "main"), main_before);
    assert_eq!(remote_tip(&remote, "other"), None);
    assert_eq!(remote_tip(&remote, &view.branch), None);
    assert!(git(&remote, &["tag"]).is_empty());
}

#[tokio::test]
async fn hooks_and_an_ssh_command_planted_in_the_repo_do_not_run_in_the_daemon() {
    let Pushing { env, remote } = pushing("");
    let users_ssh = script(env.path("user/ssh"), r#"exec sh -c "$2""#);
    let global = env.path("user/gitconfig");
    let user_config = format!(
        "[core]\n\tsshCommand = {}\n[ssh]\n\tvariant = simple\n",
        users_ssh.display()
    );
    std::fs::write(&global, user_config).unwrap();
    let repo = env.path("repos/app");
    let url = format!("fakehost:{}", remote.display());
    git(&repo, &["remote", "set-url", "origin", &url]);
    let mut daemon_command = env.daemon_command(std::time::Duration::from_secs(600));
    daemon_command
        .env("GIT_CONFIG_GLOBAL", &global)
        .env_remove("GIT_SSH_COMMAND");
    let _daemon = env.spawn_daemon(&mut daemon_command).await;
    let mut client = env.client().await;
    let (_, view, mut pane) = session(&env, &mut client, "boxed").await;
    commit(&view.worktree, "feature.rs", "feature\n");

    let ran = env.path("planted-ran");
    let outside = format!(
        r#"[ -n "$ORCH_SESSION" ] || echo "$0" >> {}"#,
        ran.display()
    );
    for hook in ["pre-push", "reference-transaction"] {
        script(repo.join(".git/hooks").join(hook), &outside);
        script(env.path("planted-hooks").join(hook), &outside);
    }
    let planted_ssh = script(
        env.path("planted-ssh"),
        &format!("echo ssh >> {}; exit 1", ran.display()),
    );
    let planted_hooks = env.path("planted-hooks");
    git(
        &view.worktree,
        &["config", "core.hooksPath", planted_hooks.to_str().unwrap()],
    );
    git(
        &view.worktree,
        &["config", "core.sshCommand", planted_ssh.to_str().unwrap()],
    );

    let push = format!("git push -u origin {}", view.branch);
    assert_eq!(shell(&mut pane, &push).await, "exit 0");
    assert_eq!(
        remote_tip(&remote, &view.branch),
        Some(git(&view.worktree, &["rev-parse", "HEAD"]))
    );
    assert_eq!(std::fs::read_to_string(&ran).ok(), None);
}

#[tokio::test]
async fn git_config_the_user_passes_in_the_environment_still_reaches_the_agent() {
    let Pushing { env, .. } = pushing("");
    let mut daemon_command = env.daemon_command(std::time::Duration::from_secs(600));
    daemon_command
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "user.name")
        .env("GIT_CONFIG_VALUE_0", "From The User");
    let _daemon = env.spawn_daemon(&mut daemon_command).await;
    let mut client = env.client().await;
    let (_, _, mut pane) = session(&env, &mut client, "boxed").await;

    let check = r#"test "$(git config user.name)" = "From The User""#;
    assert_eq!(shell(&mut pane, check).await, "exit 0");
}

async fn answer_push(client: &mut TestClient, id: &SessionId, choice: GuardChoice) {
    let asking = client
        .until(id, "a push prompt", |view| !view.guard_prompts.is_empty())
        .await;
    let prompt = &asking.guard_prompts[0];
    assert_eq!(prompt.kind, GuardKindView::Push);
    assert_eq!(prompt.target, asking.branch);
    let reply = client
        .request(Request::AnswerGuard {
            session: id.clone(),
            guard: prompt.id,
            choice,
        })
        .await;
    assert_eq!(reply, Ok(Reply::Done));
    client
        .until(id, "the prompt answered", |view| {
            view.guard_prompts.is_empty()
        })
        .await;
}

#[tokio::test]
async fn an_ask_session_prompts_before_each_push_and_a_denial_pushes_nothing() {
    let Pushing { env, remote } = pushing(ALL_SANDBOXED);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, view, mut pane) = session(&env, &mut client, "ask").await;
    commit(&view.worktree, "feature.rs", "feature\n");
    let push = format!("git push -u origin {}", view.branch);

    pane.type_line(&format!("sh {push}")).await;
    answer_push(&mut client, &id, GuardChoice::Deny).await;
    assert_ne!(shell_result(&mut pane, &push).await, "exit 0");
    pane.wait_for_text("denied").await;
    assert_eq!(remote_tip(&remote, &view.branch), None);

    pane.type_line(&format!("sh {push} ")).await;
    answer_push(&mut client, &id, GuardChoice::AllowOnce).await;
    assert_eq!(shell_result(&mut pane, &format!("{push} ")).await, "exit 0");
    assert!(remote_tip(&remote, &view.branch).is_some());
}

#[tokio::test]
async fn a_plan_session_prompts_before_a_push() {
    let Pushing { env, remote } = pushing(ALL_SANDBOXED);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, view, mut pane) = session(&env, &mut client, "plan").await;
    commit(&view.worktree, "feature.rs", "feature\n");
    let push = format!("git push -u origin {}", view.branch);

    pane.type_line(&format!("sh {push}")).await;
    answer_push(&mut client, &id, GuardChoice::AllowOnce).await;
    assert_eq!(shell_result(&mut pane, &push).await, "exit 0");
    assert!(remote_tip(&remote, &view.branch).is_some());
}

#[tokio::test]
async fn an_unsandboxed_agent_pushes_straight_to_origin_as_before() {
    let Pushing { env, remote } = pushing("");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (_, view, mut pane) = session(&env, &mut client, "inherit").await;
    commit(&view.worktree, "feature.rs", "feature\n");

    let push = "git push origin HEAD:refs/heads/other";
    assert_eq!(shell(&mut pane, push).await, "exit 0");
    assert_eq!(
        remote_tip(&remote, "other"),
        Some(git(&view.worktree, &["rev-parse", "HEAD"]))
    );
    assert!(!pane.text().contains("Broker"));
}
