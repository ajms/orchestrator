use std::process::{Command, Output};

use orch_protocol::{PhaseView, Reply, Request};

use crate::common::*;

async fn run(mut command: Command) -> Output {
    let running = tokio::task::spawn_blocking(move || command.output().unwrap());
    tokio::time::timeout(WAIT, running)
        .await
        .expect("the command finished")
        .unwrap()
}

async fn orch_with_input(env: &Env, args: &[&str], input: &str) -> Output {
    use std::io::Write;
    let mut command = env.orch();
    command
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let input = input.to_string();
    let running = tokio::task::spawn_blocking(move || {
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    });
    tokio::time::timeout(WAIT, running)
        .await
        .expect("the command finished")
        .unwrap()
}

async fn orch(env: &Env, args: &[&str]) -> Output {
    let mut command = env.orch();
    command.args(args);
    run(command).await
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[tokio::test]
async fn doctor_without_findings_says_so_and_succeeds() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;

    let output = orch(&env, &["doctor"]).await;

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("No problems found"),
        "{}",
        stdout(&output)
    );
}

#[tokio::test]
async fn doctor_lists_findings_per_repo_with_their_fixes_and_fails() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Doctor").await;
    let repo = client.sessions[&id].repo.clone();
    git(&repo, &["branch", "orch/lonely"]);

    let output = orch(&env, &["doctor"]).await;

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let report = stdout(&output);
    assert!(report.contains(&repo.display().to_string()), "{report}");
    assert!(report.contains("Leftover Branch orch/lonely"), "{report}");
    assert!(report.contains("Adopt as a Session"), "{report}");
}

#[tokio::test]
async fn doctor_prints_the_report_as_json_on_request() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Doctor").await;
    let repo = client.sessions[&id].repo.clone();
    git(&repo, &["branch", "orch/lonely"]);

    let output = orch(&env, &["doctor", "--json"]).await;

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let report: orch_protocol::ReconcileReport = serde_json::from_slice(&output.stdout).unwrap();
    let leftovers: Vec<_> = report.repo(&repo).unwrap().leftovers().cloned().collect();
    assert_eq!(
        leftovers,
        vec![orch_protocol::LeftoverView::Branch {
            branch: "orch/lonely".into()
        }]
    );
}

#[tokio::test]
async fn doctor_starts_the_daemon_when_none_is_running() {
    let env = Env::new();

    let mut command = env.orch();
    command.arg("doctor").env_remove("DBUS_SESSION_BUS_ADDRESS");
    let output = run(command).await;

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(env.socket().exists());
    orch_protocol::stop_daemon(&env.runtime_dir(), WAIT)
        .await
        .unwrap();
    assert!(!env.socket().exists());
}

async fn discard(client: &mut TestClient, id: &orch_core::SessionId) {
    let reply = client
        .request(Request::Discard {
            session: id.clone(),
            skip_teardown: false,
        })
        .await;
    assert_eq!(reply, Ok(Reply::Done));
    client
        .until(id, "Discarded", |view| view.phase == PhaseView::Discarded)
        .await;
}

#[tokio::test]
async fn forgetting_a_repo_with_live_sessions_is_refused() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Still here").await;
    let repo = client.sessions[&id].repo.clone();

    let output = orch(&env, &["repo", "forget", repo.to_str().unwrap()]).await;

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("Discard them first"),
        "{}",
        stderr(&output)
    );
    assert!(client.reconcile().await.repo(&repo).is_some());
}

#[tokio::test]
async fn forgetting_a_repo_without_live_sessions_drops_its_registration() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Gone soon").await;
    let repo = client.sessions[&id].repo.clone();
    discard(&mut client, &id).await;

    let output = orch(&env, &["repo", "forget", repo.to_str().unwrap()]).await;

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(client.reconcile().await.repo(&repo).is_none());
    assert!(repo.is_dir(), "forgetting never touches the Repo itself");
}

#[tokio::test]
async fn moving_a_repo_whose_agents_are_running_is_refused() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Busy").await;
    let repo = client.sessions[&id].repo.clone();
    let elsewhere = env.repo("elsewhere");

    let output = orch(
        &env,
        &[
            "repo",
            "move",
            repo.to_str().unwrap(),
            elsewhere.to_str().unwrap(),
        ],
    )
    .await;

    assert!(!output.status.success());
    assert!(stderr(&output).contains("running"), "{}", stderr(&output));
    assert_eq!(client.sessions[&id].repo, repo);
}

#[tokio::test]
async fn a_relocated_repo_is_moved_with_its_sessions_and_stale_overrides_are_named() {
    let env = Env::new();
    let old = env.repo("app");
    env.write_config(&format!("[repos.{old:?}]\nbase = \"main\"\n"));
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Relocate").await;
    let worktree = client.sessions[&id].worktree.clone();
    let holder = client.holder_pid(&id).unwrap();
    drop(client);
    daemon.kill();
    kill(holder, "-KILL");
    wait_until("the Holder to die", || process_gone(holder)).await;
    let new = env.path("repos/moved");
    std::fs::rename(&old, &new).unwrap();
    let new = new.canonicalize().unwrap();
    let _daemon = env.start_daemon().await;

    let output = orch(
        &env,
        &["repo", "move", old.to_str().unwrap(), new.to_str().unwrap()],
    )
    .await;

    assert!(output.status.success(), "{}", stderr(&output));
    let warning = stderr(&output);
    assert!(
        warning.contains(&format!("{:?}", old.display().to_string())),
        "{warning}"
    );
    let mut client = env.client().await;
    let moved_worktree = new.join(worktree.strip_prefix(&old).unwrap());
    client
        .until(&id, "the Session follows its Repo", |view| {
            view.repo == new && view.worktree == moved_worktree && !view.flags.repo_missing
        })
        .await;
    let report = client.reconcile().await;
    assert!(report.repo(&old).is_none());
    assert!(report.repo(&new).is_some_and(|repo| !repo.missing));
    git(&moved_worktree, &["status", "--short"]);
}

#[tokio::test]
async fn the_daemon_lists_known_repos_most_recently_used_first() {
    let env = Env::new();
    let first = env.repo("first");
    let second = env.repo("second");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    for repo in [&first, &second] {
        client
            .create(orch_protocol::CreateSession::new(repo, "Work"))
            .await;
    }

    let repos = client.request(Request::Repos).await;

    assert_eq!(
        repos,
        Ok(Reply::Repos {
            repos: vec![second.clone(), first.clone()]
        })
    );
    std::fs::rename(&first, env.path("repos/gone")).unwrap();
    assert_eq!(
        client.request(Request::Repos).await,
        Ok(Reply::Repos {
            repos: vec![second]
        })
    );
}

fn unit_file(env: &Env) -> std::path::PathBuf {
    env.path("config/systemd/user/orch-daemon.service")
}

#[tokio::test]
async fn daemon_install_writes_a_user_unit_running_this_binary_without_idle_exit() {
    let env = Env::new();

    let output = orch(&env, &["daemon", "install"]).await;

    assert!(output.status.success(), "{}", stderr(&output));
    let unit = std::fs::read_to_string(unit_file(&env)).unwrap();
    let program = std::path::Path::new(env!("CARGO_BIN_EXE_orch"))
        .canonicalize()
        .unwrap();
    assert!(
        unit.contains(&format!(
            "ExecStart={} daemon --no-idle-exit\n",
            program.display()
        )),
        "{unit}"
    );
    assert!(unit.contains("KillMode=process\n"), "{unit}");
    assert!(unit.contains("WantedBy=default.target\n"), "{unit}");
    assert!(
        stdout(&output).contains("systemctl --user enable --now orch-daemon"),
        "{}",
        stdout(&output)
    );
    assert!(!env.socket().exists(), "installing starts no Daemon");
}

#[tokio::test]
async fn daemon_uninstall_removes_the_user_unit() {
    let env = Env::new();
    assert!(orch(&env, &["daemon", "install"]).await.status.success());

    let output = orch(&env, &["daemon", "uninstall"]).await;

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!unit_file(&env).exists());
    assert!(stdout(&output).contains("systemctl --user disable --now orch-daemon"));
}

#[tokio::test]
async fn a_daemon_started_without_idle_exit_serves_clients() {
    let env = Env::new();
    let mut command = env.orch();
    command
        .args(["daemon", "--no-idle-exit", "--notify-log"])
        .arg(env.notify_log());
    let mut daemon = command.spawn().unwrap();
    wait_until("daemon socket", || env.socket().exists()).await;

    let mut client = env.client().await;
    assert!(client.session_list().await.is_empty());
    let _ = daemon.kill();
    let _ = daemon.wait();
}

#[tokio::test]
async fn orch_without_a_terminal_explains_that_the_tui_needs_one() {
    let env = Env::new();

    let output = orch(&env, &[]).await;

    assert_eq!(output.status.code(), Some(2));
    let message = stderr(&output);
    assert!(message.contains("needs a terminal"), "{message}");
    assert!(!message.contains("panicked"), "{message}");
    assert!(!env.socket().exists(), "no Daemon is spawned for nothing");
}

#[tokio::test]
async fn forgetting_a_missing_repo_whose_agents_still_run_is_refused() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Still running").await;
    let repo = client.sessions[&id].repo.clone();
    std::fs::rename(&repo, env.path("repos/elsewhere")).unwrap();

    let output = orch(&env, &["repo", "forget", repo.to_str().unwrap()]).await;

    assert!(!output.status.success());
    assert!(stderr(&output).contains("running"), "{}", stderr(&output));
    settled(&mut client).await;
    assert!(client.sessions[&id].phase.is_live());
    assert!(client.holder_pid(&id).is_some_and(|pid| !process_gone(pid)));
}

#[tokio::test]
async fn a_repo_already_moved_on_disk_is_moved_while_its_agents_keep_running() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Keep going").await;
    let old = client.sessions[&id].repo.clone();
    let worktree = client.sessions[&id].worktree.clone();
    let holder = client.holder_pid(&id).unwrap();
    std::fs::rename(&old, env.path("repos/moved")).unwrap();
    let new = env.path("repos/moved").canonicalize().unwrap();

    let output = orch(
        &env,
        &["repo", "move", old.to_str().unwrap(), new.to_str().unwrap()],
    )
    .await;

    assert!(output.status.success(), "{}", stderr(&output));
    let moved_worktree = new.join(worktree.strip_prefix(&old).unwrap());
    client
        .until(&id, "the Session follows its Repo", |view| {
            view.repo == new && view.worktree == moved_worktree
        })
        .await;
    assert_eq!(client.holder_pid(&id), Some(holder));
    assert!(!process_gone(holder));
    git(&moved_worktree, &["status", "--short"]);
}

#[tokio::test]
async fn a_repo_is_found_by_its_real_path_when_moved_through_a_symlink() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Linked").await;
    let old = client.sessions[&id].repo.clone();
    std::os::unix::fs::symlink(env.path("repos"), env.path("link")).unwrap();
    std::fs::rename(&old, env.path("repos/moved")).unwrap();
    let through_link = env.path("link/app");

    let output = orch(
        &env,
        &[
            "repo",
            "move",
            through_link.to_str().unwrap(),
            env.path("link/moved").to_str().unwrap(),
        ],
    )
    .await;

    assert!(output.status.success(), "{}", stderr(&output));
    let new = env.path("repos/moved").canonicalize().unwrap();
    client
        .until(&id, "the Session follows its Repo", |view| view.repo == new)
        .await;
}

#[tokio::test]
async fn an_unreadable_config_is_reported_when_checking_stale_overrides() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Broken config").await;
    let old = client.sessions[&id].repo.clone();
    std::fs::rename(&old, env.path("repos/moved")).unwrap();
    std::fs::write(env.path("config/orchestrator/config.toml"), "not = [valid").unwrap();

    let output = orch(
        &env,
        &[
            "repo",
            "move",
            old.to_str().unwrap(),
            env.path("repos/moved").to_str().unwrap(),
        ],
    )
    .await;

    assert!(output.status.success(), "{}", stderr(&output));
    let warning = stderr(&output);
    assert!(warning.contains("could not check"), "{warning}");
    assert!(warning.contains("config.toml"), "{warning}");
}

fn untrusted_repo(env: &Env) -> std::path::PathBuf {
    let repo = env.repo("app");
    commit(&repo, ".orchestrator.toml", "setup = \"touch ran\"\n");
    repo
}

async fn creatable(client: &mut TestClient, repo: &std::path::Path) -> bool {
    let create = orch_protocol::CreateSession::new(repo, "Try");
    !matches!(
        client.request(Request::CreateSession(create)).await,
        Err(orch_protocol::RequestError::Untrusted { .. })
    )
}

#[tokio::test]
async fn trust_shows_the_items_and_approves_after_a_yes() {
    let env = Env::new();
    let repo = untrusted_repo(&env);
    let _daemon = env.start_daemon().await;

    let output = orch_with_input(&env, &["trust", repo.to_str().unwrap()], "y\n").await;

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("Setup script: touch ran"),
        "{}",
        stdout(&output)
    );
    let mut client = env.client().await;
    assert!(creatable(&mut client, &repo).await);
}

#[tokio::test]
async fn trust_approves_nothing_without_a_yes() {
    let env = Env::new();
    let repo = untrusted_repo(&env);
    let _daemon = env.start_daemon().await;

    let output = orch_with_input(&env, &["trust", repo.to_str().unwrap()], "\n").await;

    assert_eq!(output.status.code(), Some(1));
    let mut client = env.client().await;
    assert!(!creatable(&mut client, &repo).await);
}

#[tokio::test]
async fn trust_with_yes_approves_without_asking() {
    let env = Env::new();
    let repo = untrusted_repo(&env);
    let _daemon = env.start_daemon().await;

    let output = orch(&env, &["trust", "--yes", repo.to_str().unwrap()]).await;

    assert!(output.status.success(), "{}", stderr(&output));
    let mut client = env.client().await;
    assert!(creatable(&mut client, &repo).await);
}

#[tokio::test]
async fn trust_says_so_when_nothing_needs_it() {
    let env = Env::new();
    let repo = env.repo("plain");
    let _daemon = env.start_daemon().await;

    let output = orch(&env, &["trust", repo.to_str().unwrap()]).await;

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("needs no Trust"),
        "{}",
        stdout(&output)
    );
}

#[tokio::test]
async fn daemon_uninstall_removes_the_enable_symlink_left_behind() {
    let env = Env::new();
    assert!(orch(&env, &["daemon", "install"]).await.status.success());
    let wants = env.path("config/systemd/user/default.target.wants");
    std::fs::create_dir_all(&wants).unwrap();
    std::os::unix::fs::symlink(unit_file(&env), wants.join("orch-daemon.service")).unwrap();

    let output = orch(&env, &["daemon", "uninstall"]).await;

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(std::fs::symlink_metadata(wants.join("orch-daemon.service")).is_err());
}

#[tokio::test]
async fn no_idle_exit_and_an_idle_timeout_conflict() {
    let env = Env::new();
    let output = orch(
        &env,
        &["daemon", "--no-idle-exit", "--idle-timeout-ms", "10"],
    )
    .await;
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("cannot be used with"),
        "{}",
        stderr(&output)
    );
}
