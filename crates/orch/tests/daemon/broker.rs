use std::path::PathBuf;

use orch_core::SessionId;
use orch_protocol::{
    CreateSession, GuardChoice, GuardKindView, PhaseView, Reply, Request, SessionView,
};

use crate::common::*;

struct Pushing {
    env: Env,
    remote: PathBuf,
}

fn pushing() -> Pushing {
    let env = Env::new();
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
    let Pushing { env, remote } = pushing();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, view, mut pane) = session(&env, &mut client, "inherit").await;
    commit(&view.worktree, "feature.rs", "feature\n");

    let push = format!("git push -u origin {}", view.branch);
    assert_eq!(shell(&mut pane, &push).await, "exit 0");
    assert_eq!(
        remote_tip(&remote, &view.branch),
        Some(git(&view.worktree, &["rev-parse", "HEAD"]))
    );
    client.until_notice(&id, "Broker: pushed").await;
    assert!(!pane.text().contains("Broker:"), "{}", pane.text());
}

#[tokio::test]
async fn a_push_that_reached_origin_succeeds_even_if_tracking_cannot_be_written() {
    let Pushing { env, remote } = pushing();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, view, mut pane) = session(&env, &mut client, "inherit").await;
    commit(&view.worktree, "feature.rs", "feature\n");
    let lock = env.path("repos/app/.git/config.lock");
    std::fs::write(&lock, "").unwrap();

    let push = format!("git push origin {}", view.branch);
    assert_eq!(shell(&mut pane, &push).await, "exit 0");
    std::fs::remove_file(&lock).unwrap();
    assert_eq!(
        remote_tip(&remote, &view.branch),
        Some(git(&view.worktree, &["rev-parse", "HEAD"]))
    );
    client.until_notice(&id, "Broker: pushed").await;
}

#[tokio::test]
async fn a_rewritten_branch_goes_through_with_a_lease_but_not_with_a_plain_force() {
    let Pushing { env, remote } = pushing();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, view, mut pane) = session(&env, &mut client, "inherit").await;
    commit(&view.worktree, "feature.rs", "feature\n");
    let push = format!("git push -u origin {}", view.branch);
    assert_eq!(shell(&mut pane, &push).await, "exit 0");
    git(
        &view.worktree,
        &["commit", "-q", "--amend", "-m", "reworded"],
    );
    let rewritten = git(&view.worktree, &["rev-parse", "HEAD"]);

    assert_ne!(shell(&mut pane, "git push --force").await, "exit 0");
    client.until_notice(&id, "--force-with-lease").await;
    assert_ne!(remote_tip(&remote, &view.branch), Some(rewritten.clone()));

    assert_eq!(
        shell(&mut pane, "git push --force-with-lease").await,
        "exit 0"
    );
    assert_eq!(remote_tip(&remote, &view.branch), Some(rewritten));
}

#[tokio::test]
async fn other_refs_tags_and_the_base_branch_are_refused() {
    let Pushing { env, remote } = pushing();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, view, mut pane) = session(&env, &mut client, "inherit").await;
    commit(&view.worktree, "feature.rs", "feature\n");
    let main_before = remote_tip(&remote, "main");

    assert_ne!(
        shell(&mut pane, "git push origin HEAD:main").await,
        "exit 0"
    );
    client
        .until_notice(&id, "never pushes the Base branch main")
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
    let Pushing { env, remote } = pushing();
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
    let (_, view, mut pane) = session(&env, &mut client, "inherit").await;
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
    let Pushing { env, .. } = pushing();
    let mut daemon_command = env.daemon_command(std::time::Duration::from_secs(600));
    daemon_command
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "user.name")
        .env("GIT_CONFIG_VALUE_0", "From The User");
    let _daemon = env.spawn_daemon(&mut daemon_command).await;
    let mut client = env.client().await;
    let (_, _, mut pane) = session(&env, &mut client, "inherit").await;

    let check = r#"test "$(git config user.name)" = "From The User""#;
    assert_eq!(shell(&mut pane, check).await, "exit 0");
}

async fn answer_push(client: &mut TestClient, id: &SessionId, choice: GuardChoice) {
    answer_broker(client, id, GuardKindView::Push, choice).await;
}

async fn answer_pr_change(client: &mut TestClient, id: &SessionId, choice: GuardChoice) {
    answer_broker(client, id, GuardKindView::PullRequest, choice).await;
}

async fn answer_issue_change(client: &mut TestClient, id: &SessionId, choice: GuardChoice) {
    answer_broker(client, id, GuardKindView::Issue, choice).await;
}

async fn answer_broker(
    client: &mut TestClient,
    id: &SessionId,
    kind: GuardKindView,
    choice: GuardChoice,
) {
    let asking = client
        .until(id, "a Broker prompt", |view| !view.guard_prompts.is_empty())
        .await;
    let prompt = &asking.guard_prompts[0];
    assert_eq!(prompt.kind, kind);
    match kind {
        GuardKindView::Issue => assert_eq!(prompt.target, "acme/app"),
        _ => assert_eq!(prompt.target, asking.branch),
    }
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
    let Pushing { env, remote } = pushing();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, view, mut pane) = session(&env, &mut client, "ask").await;
    commit(&view.worktree, "feature.rs", "feature\n");
    let push = format!("git push -u origin {}", view.branch);

    pane.type_line(&format!("sh {push}")).await;
    answer_push(&mut client, &id, GuardChoice::Deny).await;
    assert_ne!(shell_result(&mut pane, &push).await, "exit 0");
    client.until_notice(&id, "denied").await;
    assert_eq!(remote_tip(&remote, &view.branch), None);

    pane.type_line(&format!("sh {push} ")).await;
    answer_push(&mut client, &id, GuardChoice::AllowOnce).await;
    assert_eq!(shell_result(&mut pane, &format!("{push} ")).await, "exit 0");
    assert!(remote_tip(&remote, &view.branch).is_some());
}

#[tokio::test]
async fn a_plan_session_prompts_before_a_push() {
    let Pushing { env, remote } = pushing();
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
    let Pushing { env, remote } = pushing();
    env.write_config(OPEN_PRESET);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (_, view, mut pane) = session(&env, &mut client, "open").await;
    commit(&view.worktree, "feature.rs", "feature\n");

    let push = "git push origin HEAD:refs/heads/other";
    assert_eq!(shell(&mut pane, push).await, "exit 0");
    assert_eq!(
        remote_tip(&remote, "other"),
        Some(git(&view.worktree, &["rev-parse", "HEAD"]))
    );
    assert!(!pane.text().contains("Broker"));
}

fn on_github() -> Env {
    let env = Env::new();
    env.fake_gh();
    let repo = env.repo("app");
    git(
        &repo,
        &["remote", "add", "origin", "git@github.com:acme/app.git"],
    );
    env
}

fn gh_calls(env: &Env) -> Vec<Vec<String>> {
    let repo = env.path("repos/app").canonicalize().unwrap();
    let mut calls: Vec<Vec<String>> = Vec::new();
    for line in env.gh_calls().lines() {
        match std::path::Path::new(line) == repo {
            true => calls.push(Vec::new()),
            false => calls.last_mut().unwrap().push(line.into()),
        }
    }
    calls
}

fn args(line: &str) -> Vec<String> {
    line.split(' ').map(String::from).collect()
}

#[tokio::test]
async fn a_sandboxed_agent_reads_its_repo_through_gh() {
    let env = on_github();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, view, mut pane) = session(&env, &mut client, "inherit").await;
    let out = view.worktree.join("out");

    let read = format!("gh issue view 7 --json title -q .title > {}", out.display());
    assert_eq!(shell(&mut pane, &read).await, "exit 0");
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "issue 7\n");
    assert_eq!(
        gh_calls(&env),
        [args(
            "issue view 7 --json title --jq .title --repo acme/app"
        )]
    );
    client.until_notice(&id, "Broker: gh issue view 7").await;
}

#[tokio::test]
async fn a_sandboxed_agent_creates_and_edits_its_own_pr_with_the_base_fixed() {
    let env = on_github();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, view, mut pane) = session(&env, &mut client, "inherit").await;
    let (body, out) = (view.worktree.join("body.md"), view.worktree.join("out"));
    std::fs::write(&body, "Why").unwrap();
    let branch = view.branch.as_str();

    let create = format!(
        "gh pr create -t Title --body-file {} --base main --head {branch} > {}",
        body.display(),
        out.display()
    );
    assert_eq!(shell(&mut pane, &create).await, "exit 0");
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        "https://github.com/acme/app/pull/42\n"
    );
    env.gh_lists(r#"[{"number":42}]"#);
    assert_eq!(
        shell(&mut pane, "gh pr edit 42 --title=New").await,
        "exit 0"
    );
    assert_eq!(shell(&mut pane, "gh pr ready --undo").await, "exit 0");

    let calls = gh_calls(&env);
    let fixed = |line: &str| args(&line.replace("BRANCH", branch));
    let mut created = fixed("pr create --head BRANCH --base main --title Title --body");
    created.extend(["Why".into(), "--repo".into(), "acme/app".into()]);
    assert_eq!(calls[0], created);
    assert_eq!(
        calls.last().unwrap(),
        &fixed("pr ready BRANCH --undo --repo acme/app")
    );
    assert!(calls.contains(&fixed("pr edit BRANCH --title New --repo acme/app")));
    client.until_notice(&id, "Broker: gh pr ready").await;
}

#[tokio::test]
async fn merges_other_repos_and_a_changed_base_are_refused_with_a_message() {
    let env = on_github();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, view, mut pane) = session(&env, &mut client, "inherit").await;
    let err = view.worktree.join("err");

    for (call, says) in [
        ("gh pr merge 42", "doesn't run `gh pr merge`"),
        ("gh pr comment 42 -b hi", "doesn't run `gh pr comment`"),
        ("gh issue view 1 -R other/app", "not other/app"),
        (
            "gh pr create --base develop",
            "fixed to the Base branch main",
        ),
        (
            "gh pr edit 42 --base develop",
            "fixed to the Base branch main",
        ),
        ("gh api -X POST repos/acme/app/issues", "only GET"),
        ("gh api repos/other/app/issues", "only repos/acme/app"),
    ] {
        let refused = format!("{call} 2> {}", err.display());
        assert_ne!(shell(&mut pane, &refused).await, "exit 0", "{call}");
        let message = std::fs::read_to_string(&err).unwrap();
        assert!(message.contains(says), "{call}: {message}");
    }
    assert_eq!(gh_calls(&env), Vec::<Vec<String>>::new());
    client
        .until_notice(&id, "Broker refused: the Broker doesn't run `gh pr merge`")
        .await;
    assert!(!pane.text().contains("Broker refused"), "{}", pane.text());
}

#[tokio::test]
async fn gh_auth_answers_as_logged_out_without_a_notice() {
    let env = on_github();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, view, mut pane) = session(&env, &mut client, "inherit").await;
    let err = view.worktree.join("err");

    for call in ["gh auth token", "gh auth status"] {
        let quiet = format!("{call} 2> {}", err.display());
        assert_eq!(shell(&mut pane, &quiet).await, "exit 1", "{call}");
        let message = std::fs::read_to_string(&err).unwrap();
        assert!(message.contains("not logged in"), "{call}: {message}");
        assert_eq!(message.lines().count(), 1, "{call}: {message}");
    }
    assert_ne!(shell(&mut pane, "gh pr merge 42").await, "exit 0");
    client.until_notice(&id, "`gh pr merge`").await;

    assert_eq!(client.notices.len(), 1, "{:?}", client.notices);
    assert_eq!(gh_calls(&env), Vec::<Vec<String>>::new());
}

#[tokio::test]
async fn an_ask_session_prompts_before_a_pr_change_but_never_for_a_read() {
    let env = on_github();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _, mut pane) = session(&env, &mut client, "ask").await;

    assert_eq!(shell(&mut pane, "gh pr list").await, "exit 0");
    let create = "gh pr create --fill";
    pane.type_line(&format!("sh {create}")).await;
    answer_pr_change(&mut client, &id, GuardChoice::Deny).await;
    assert_ne!(shell_result(&mut pane, create).await, "exit 0");
    assert_eq!(gh_calls(&env).len(), 1);

    pane.type_line(&format!("sh {create} ")).await;
    answer_pr_change(&mut client, &id, GuardChoice::AllowOnce).await;
    assert_eq!(
        shell_result(&mut pane, &format!("{create} ")).await,
        "exit 0"
    );
    assert_eq!(gh_calls(&env)[1][..2], args("pr create"));
}

#[tokio::test]
async fn a_plan_session_prompts_before_a_pr_edit() {
    let env = on_github();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _, mut pane) = session(&env, &mut client, "plan").await;

    let edit = "gh pr edit --body Better";
    pane.type_line(&format!("sh {edit}")).await;
    answer_pr_change(&mut client, &id, GuardChoice::AllowOnce).await;
    assert_eq!(shell_result(&mut pane, edit).await, "exit 0");
    assert_eq!(gh_calls(&env)[0][..2], args("pr edit"));
}

#[tokio::test]
async fn a_pr_the_agent_creates_puts_the_session_in_pr_open() {
    let env = on_github();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _, mut pane) = session(&env, &mut client, "inherit").await;

    assert_eq!(
        shell(&mut pane, "gh pr create -t Title -b Why").await,
        "exit 0"
    );

    let view = client
        .until(&id, "PR open", |view| view.phase == PhaseView::PrOpen)
        .await;
    assert_eq!(view.flags.pr_number, Some(42));
}

#[tokio::test]
async fn a_sandboxed_agent_files_comments_on_and_links_issues_in_its_repo() {
    let env = on_github();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, view, mut pane) = session(&env, &mut client, "inherit").await;
    let (body, out) = (view.worktree.join("body.md"), view.worktree.join("out"));
    std::fs::write(&body, "Why").unwrap();

    let create = format!(
        "gh issue create -t Bug --body-file {} -l bug > {}",
        body.display(),
        out.display()
    );
    assert_eq!(shell(&mut pane, &create).await, "exit 0");
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        "https://github.com/acme/app/issues/8\n"
    );
    let comment = format!("gh issue comment 8 -F {}", body.display());
    assert_eq!(shell(&mut pane, &comment).await, "exit 0");
    let link = "gh api -X POST repos/acme/app/issues/3/sub_issues -F sub_issue_id=99";
    assert_eq!(shell(&mut pane, link).await, "exit 0");
    assert_ne!(shell(&mut pane, "gh issue delete 8 --yes").await, "exit 0");

    assert_eq!(
        gh_calls(&env),
        [
            args("issue create --title Bug --body Why --label bug --repo acme/app"),
            args("issue comment 8 --body Why --repo acme/app"),
            args("api repos/acme/app/issues/3/sub_issues --method POST --field sub_issue_id=99"),
        ]
    );
    client.until_notice(&id, "Broker: gh issue create").await;
}

#[tokio::test]
async fn an_ask_session_prompts_before_an_issue_change_and_once_for_the_session() {
    let env = on_github();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _, mut pane) = session(&env, &mut client, "ask").await;

    let comment = "gh issue comment 8 -b hi";
    pane.type_line(&format!("sh {comment}")).await;
    answer_issue_change(&mut client, &id, GuardChoice::Deny).await;
    assert_ne!(shell_result(&mut pane, comment).await, "exit 0");
    assert_eq!(gh_calls(&env).len(), 0);

    pane.type_line(&format!("sh {comment} ")).await;
    answer_issue_change(&mut client, &id, GuardChoice::AllowForSession).await;
    assert_eq!(
        shell_result(&mut pane, &format!("{comment} ")).await,
        "exit 0"
    );
    assert_eq!(shell(&mut pane, "gh issue close 8").await, "exit 0");
    assert_eq!(gh_calls(&env).len(), 2);
}

fn signing_key(env: &Env) -> (PathBuf, String) {
    let key = env.path("keys/orch");
    std::fs::create_dir_all(key.parent().unwrap()).unwrap();
    let made = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "orch", "-f"])
        .arg(&key)
        .status()
        .unwrap();
    assert!(made.success());
    let public = std::fs::read_to_string(key.with_extension("pub")).unwrap();
    (key, public.trim().into())
}

fn with_signing_key(env: &Env, key: &std::path::Path) {
    env.write_config(&format!(
        "[signing]\nkey = {:?}\n",
        key.display().to_string()
    ));
}

fn last_commit(worktree: &std::path::Path, format: &str) -> String {
    git(worktree, &["log", "-1", &format!("--format={format}")])
}

async fn signing_daemon(env: &Env) -> Daemon {
    let global = env.path("user/gitconfig");
    std::fs::create_dir_all(global.parent().unwrap()).unwrap();
    std::fs::write(&global, "[commit]\n\tgpgsign = true\n").unwrap();
    let mut daemon_command = env.daemon_command(std::time::Duration::from_secs(600));
    daemon_command.env("GIT_CONFIG_GLOBAL", &global);
    env.spawn_daemon(&mut daemon_command).await
}

#[tokio::test]
async fn without_a_signing_key_a_sandboxed_commit_goes_through_unsigned_as_the_agent() {
    let Pushing { env, .. } = pushing();
    let _daemon = signing_daemon(&env).await;
    let mut client = env.client().await;
    let (_, view, mut pane) = session(&env, &mut client, "inherit").await;

    let made = shell(&mut pane, "git commit -q --allow-empty -m work").await;
    assert_eq!(made, "exit 0");
    assert_eq!(
        last_commit(&view.worktree, "%an|%cn|%ce|%G?"),
        "Test (orch)|Test (orch)|test@example.com|N"
    );
}

#[tokio::test]
async fn with_a_signing_key_the_broker_signs_a_sandboxed_commit() {
    let Pushing { env, .. } = pushing();
    let (key, public) = signing_key(&env);
    with_signing_key(&env, &key);
    let _daemon = signing_daemon(&env).await;
    let mut client = env.client().await;
    let (_, view, mut pane) = session(&env, &mut client, "inherit").await;

    let made = shell(&mut pane, "git commit -q --allow-empty -m work").await;
    assert_eq!(made, "exit 0");
    let signers = env.path("keys/signers");
    std::fs::write(&signers, format!("test@example.com {public}\n")).unwrap();
    let signers = format!("gpg.ssh.allowedSignersFile={}", signers.display());
    git(&view.worktree, &["-c", &signers, "verify-commit", "HEAD"]);
    assert_eq!(last_commit(&view.worktree, "%cn"), "Test (orch)");
}

#[tokio::test]
async fn the_broker_signs_a_commit_whose_message_is_not_utf8() {
    let Pushing { env, .. } = pushing();
    let (key, public) = signing_key(&env);
    with_signing_key(&env, &key);
    let _daemon = signing_daemon(&env).await;
    let mut client = env.client().await;
    let (_, view, mut pane) = session(&env, &mut client, "inherit").await;

    let latin1 = r"printf 'caf\351\n' > ../msg && git -c i18n.commitEncoding=latin1 commit -q --allow-empty -F ../msg";
    assert_eq!(shell(&mut pane, latin1).await, "exit 0");
    let signers = env.path("keys/signers");
    std::fs::write(&signers, format!("test@example.com {public}\n")).unwrap();
    let signers = format!("gpg.ssh.allowedSignersFile={}", signers.display());
    git(&view.worktree, &["-c", &signers, "verify-commit", "HEAD"]);
}

#[tokio::test]
async fn the_broker_refuses_to_sign_a_commit_by_another_committer() {
    let Pushing { env, .. } = pushing();
    let (key, _) = signing_key(&env);
    with_signing_key(&env, &key);
    let _daemon = signing_daemon(&env).await;
    let mut client = env.client().await;
    let (id, view, mut pane) = session(&env, &mut client, "inherit").await;
    let before = last_commit(&view.worktree, "%H");

    let forged = "GIT_COMMITTER_NAME=Test git commit -q --allow-empty -m forged";
    assert_ne!(shell(&mut pane, forged).await, "exit 0");
    client
        .until_notice(&id, "Broker refused: the Broker signs only commits")
        .await;
    assert_eq!(last_commit(&view.worktree, "%H"), before);
}

#[tokio::test]
async fn an_unsandboxed_agent_commits_as_the_user() {
    let Pushing { env, .. } = pushing();
    env.write_config(OPEN_PRESET);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (_, view, mut pane) = session(&env, &mut client, "open").await;

    let made = shell(&mut pane, "git commit -q --allow-empty -m work").await;
    assert_eq!(made, "exit 0");
    assert_eq!(last_commit(&view.worktree, "%cn|%G?"), "Test|N");
}
