use std::path::{Path, PathBuf};
use std::time::Duration;

use orch_core::SessionId;
use orch_holder::{HolderClient, ToHolder};
use orch_protocol::{AgentStateView as State, CreateSession, PhaseView, Reply, Request};

use crate::bwrap::bwrap_runs;
use crate::common::*;

async fn sandboxed_session(env: &Env, client: &mut TestClient) -> (SessionId, PaneView) {
    let repo = env.repo("app");
    let create = CreateSession::new(&repo, "Boxed");
    let id = client.create(create).await;
    client
        .until(&id, "running", |view| view.agent.is_some())
        .await;
    let pane = env.pane(&id, PANE).await;
    (id, pane)
}

async fn run(pane: &mut PaneView, command: &str, path: &Path) -> String {
    let line = format!("{command} {}", path.display());
    pane.type_line(&line).await;
    let answer = format!("{line} -> ");
    pane.wait_for(&answer, |pane| pane.text().contains(&answer))
        .await;
    let text = pane.text();
    let start = text.find(&answer).unwrap() + answer.len();
    text[start..].lines().next().unwrap().trim().to_string()
}

async fn shell(pane: &mut PaneView, script: &str) -> String {
    run(pane, "sh", Path::new(script)).await
}

fn seed(path: PathBuf, content: &str) -> PathBuf {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    path
}

#[tokio::test]
async fn a_sandboxed_agent_writes_its_worktree_and_nothing_else_of_the_users() {
    if !bwrap_runs() {
        return;
    }
    let env = Env::new();
    env.real_bwrap();
    let settings = seed(env.path("home/.claude/settings.json"), "{}");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = sandboxed_session(&env, &mut client).await;
    let view = client.sessions[&id].clone();

    let inside = view.worktree.join("inside.txt");
    assert_eq!(run(&mut pane, "write", &inside).await, "ok");
    assert!(inside.exists());

    let stray = env.path("home/stray.txt");
    run(&mut pane, "write", &stray).await;
    assert!(!stray.exists());

    assert_ne!(run(&mut pane, "write", &settings).await, "ok");
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{}");

    let main_checkout = env.path("repos/app/README.md");
    assert_ne!(run(&mut pane, "write", &main_checkout).await, "ok");
    assert_eq!(std::fs::read_to_string(main_checkout).unwrap(), "hello\n");

    let history = env.path("home/.claude/history.jsonl");
    assert_eq!(run(&mut pane, "write", &history).await, "ok");
    assert!(history.exists());
}

#[tokio::test]
async fn a_sandboxed_agent_commits_but_cannot_change_git_hooks_or_config() {
    if !bwrap_runs() {
        return;
    }
    let env = Env::new();
    env.real_bwrap();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = sandboxed_session(&env, &mut client).await;
    let worktree = client.sessions[&id].worktree.clone();
    let git_dir = env.path("repos/app/.git");

    let commit = format!(
        "git -C {} commit -q --allow-empty -m boxed",
        worktree.display()
    );
    assert_eq!(shell(&mut pane, &commit).await, "exit 0");
    assert_eq!(git(&worktree, &["log", "-1", "--format=%s"]), "boxed");

    let hook = format!("echo evil > {}/hooks/pre-commit", git_dir.display());
    assert_ne!(shell(&mut pane, &hook).await, "exit 0");
    let config = format!("git config --file {}/config core.x y", git_dir.display());
    assert_ne!(shell(&mut pane, &config).await, "exit 0");
}

#[tokio::test]
async fn a_sandboxed_agent_cannot_plant_a_missing_skills_dir_or_git_hooks_dir() {
    if !bwrap_runs() {
        return;
    }
    let env = Env::new();
    env.real_bwrap();
    seed(env.path("home/.claude/settings.json"), "{}");
    let repo = env.repo("app");
    std::fs::remove_dir_all(repo.join(".git/hooks")).unwrap();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = client.create(CreateSession::new(&repo, "Boxed")).await;
    client
        .until(&id, "running", |view| view.agent.is_some())
        .await;
    let mut pane = env.pane(&id, PANE).await;

    let skills = env.path("home/.claude/skills");
    let skill = format!(
        "mkdir -p {0}/evil && echo evil > {0}/evil/SKILL.md",
        skills.display()
    );
    assert_ne!(shell(&mut pane, &skill).await, "exit 0");
    assert!(!skills.join("evil").exists());

    let hooks = repo.join(".git/hooks");
    let hook = format!(
        "mkdir -p {0} && echo evil > {0}/pre-commit",
        hooks.display()
    );
    assert_ne!(shell(&mut pane, &hook).await, "exit 0");
    assert!(!hooks.join("pre-commit").exists());
}

#[tokio::test]
async fn secrets_in_home_are_invisible_to_a_sandboxed_agent() {
    if !bwrap_runs() {
        return;
    }
    let env = Env::new();
    env.real_bwrap();
    seed(env.path("home/.ssh/id_ed25519"), "private key");
    seed(env.path("home/.gitconfig"), "[user]\n\tname = Dev\n");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (_, mut pane) = sandboxed_session(&env, &mut client).await;

    assert_eq!(run(&mut pane, "exists", &env.path("home/.ssh")).await, "no");
    assert_eq!(
        run(&mut pane, "exists", &env.path("home/.gitconfig")).await,
        "yes"
    );
}

#[tokio::test]
async fn claude_json_saves_from_inside_the_sandbox_persist() {
    if !bwrap_runs() {
        return;
    }
    let env = Env::new();
    env.real_bwrap();
    let config = seed(env.path("home/.claude.json"), "before");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (_, mut pane) = sandboxed_session(&env, &mut client).await;

    assert_eq!(run(&mut pane, "save", &config).await, "ok");
    assert_eq!(std::fs::read_to_string(config).unwrap(), "saved");
}

#[tokio::test]
async fn a_sandboxed_agent_reports_through_its_holder_and_is_not_asked_about_outside_writes() {
    if !bwrap_runs() {
        return;
    }
    let env = Env::new();
    env.real_bwrap();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = sandboxed_session(&env, &mut client).await;
    let worktree = client.sessions[&id].worktree.clone();

    pane.hook(&hook("SessionStart", r#""source":"startup""#))
        .await;
    client
        .until(&id, "Idle", |view| view.agent == Some(State::Idle))
        .await;
    client.history.clear();
    pane.hook(&hook(
        "PreToolUse",
        &format!(
            r#""tool_name":"Write","tool_input":{{"file_path":"/etc/hosts","content":"x"}},"cwd":"{}""#,
            worktree.display()
        ),
    ))
    .await;
    pane.type_line("print guard-settled").await;
    pane.wait_for_text("guard-settled").await;
    settled(&mut client).await;
    assert!(
        client
            .history
            .iter()
            .all(|view| view.guard_prompts.is_empty())
    );
}

#[tokio::test]
async fn a_sandboxed_agents_tmp_is_its_sessions_own_and_goes_when_the_session_is_discarded() {
    if !bwrap_runs() {
        return;
    }
    let env = Env::new();
    env.real_bwrap();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = sandboxed_session(&env, &mut client).await;
    let session_tmp = env
        .path("state/orchestrator/sessions")
        .join(id.as_str())
        .join("tmp");

    pane.type_line("env TMPDIR").await;
    pane.wait_for_text("TMPDIR=/tmp").await;
    assert_eq!(run(&mut pane, "write", Path::new("/tmp/note")).await, "ok");
    assert!(session_tmp.join("note").exists());

    let reply = client
        .request(Request::Discard {
            session: id.clone(),
            skip_teardown: true,
        })
        .await;
    assert_eq!(reply, Ok(Reply::Done));
    client
        .until(&id, "Discarded", |view| view.phase == PhaseView::Discarded)
        .await;
    wait_until("the Session's tmp to go", || !session_tmp.exists()).await;
}

#[tokio::test]
async fn an_unsandboxed_preset_runs_the_agent_as_before() {
    let env = Env::new();
    env.write_config(UNSANDBOXED);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Open").await;
    let mut pane = env.pane(&id, PANE).await;

    let stray = env.path("home/stray.txt");
    assert_eq!(run(&mut pane, "write", &stray).await, "ok");
    assert!(stray.exists());
}

#[tokio::test]
async fn a_sandboxed_agent_pushes_its_branch_through_the_broker() {
    if !bwrap_runs() {
        return;
    }
    let env = Env::new();
    env.real_bwrap();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = sandboxed_session(&env, &mut client).await;
    let view = client.sessions[&id].clone();
    let remote = env.remote(&env.path("repos/app"));

    let push = format!(
        "git commit -q --allow-empty -m boxed && git push -u origin {}",
        view.branch
    );
    assert_eq!(shell(&mut pane, &push).await, "exit 0");
    assert_eq!(
        git(&remote, &["log", "-1", "--format=%s", &view.branch]),
        "boxed"
    );
    pane.wait_for_text("Broker: pushed").await;
}

#[tokio::test]
async fn only_a_session_whose_preset_opts_out_is_marked_unsandboxed() {
    let env = Env::new();
    env.write_config(&format!(
        "[defaults.presets.mine]\nmode = \"plan\"\n\n{OPEN_PRESET}"
    ));
    let repo = env.repo("app");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;

    for (preset, unsandboxed) in [
        ("plan", false),
        ("ask", false),
        ("edits", false),
        ("auto", false),
        ("inherit", false),
        ("mine", false),
        ("open", true),
    ] {
        let mut create = CreateSession::new(&repo, preset);
        create.preset = Some(preset.into());
        let id = client.create(create).await;
        let view = client
            .until(&id, "running", |view| view.holder_pid.is_some())
            .await;
        assert_eq!(view.unsandboxed, unsandboxed, "{preset}");
    }
}

#[tokio::test]
async fn an_opted_out_session_stays_marked_once_its_holder_is_gone() {
    let env = Env::new();
    env.write_config(OPEN_PRESET);
    let repo = env.repo("app");
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let mut ids = Vec::new();
    for preset in ["open", "plan"] {
        let mut create = CreateSession::new(&repo, preset);
        create.preset = Some(preset.into());
        let id = client.create(create).await;
        let holder = client
            .until(&id, "running", |view| view.holder_pid.is_some())
            .await
            .holder_pid
            .unwrap();
        std::process::Command::new("kill")
            .args(["-9", &holder.to_string()])
            .status()
            .unwrap();
        let gone = client
            .until(&id, "the Holder gone", |view| view.holder_pid.is_none())
            .await;
        assert_eq!(gone.unsandboxed, preset == "open", "{preset}");
        ids.push((id, preset));
    }
    daemon.kill();

    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    for (id, preset) in ids {
        let view = client.until(&id, "loaded", |_| true).await;
        assert_eq!(view.unsandboxed, preset == "open", "{preset}");
    }
}

#[tokio::test]
async fn a_session_from_before_the_upgrade_resumes_sandboxed() {
    let env = Env::new();
    env.write_config(UNSANDBOXED);
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Old").await;
    let old_holder = client.holder_pid(&id).unwrap() as u32;
    assert!(client.sessions[&id].unsandboxed);
    let mut pane = env.pane(&id, PANE).await;
    pane.type_line("exit 0").await;
    client
        .until(&id, "Exited", |view| view.agent == Some(State::Exited))
        .await;
    daemon.kill();

    env.write_config("");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let reply = client
        .request(Request::Resume {
            session: id.clone(),
        })
        .await;
    assert_eq!(reply, Ok(Reply::Done));
    let resumed = client
        .until(&id, "a fresh Holder", |view| {
            view.holder_pid.is_some_and(|pid| pid != old_holder)
        })
        .await;
    assert!(!resumed.unsandboxed);
}

#[tokio::test]
#[ignore = "runs the real Claude Code in the real Sandbox: needs bwrap and a logged-in claude"]
async fn real_claude_runs_sandboxed_writes_its_worktree_and_stops_cleanly() {
    assert!(bwrap_runs(), "bubblewrap cannot run here");
    let env = Env::new();
    env.real_bwrap();
    std::fs::write(env.path("config/orchestrator/config.toml"), "").unwrap();
    let repo = env.repo("app");
    let mut command = env.daemon_command(Duration::from_secs(600));
    command.env("HOME", std::env::var_os("HOME").unwrap());
    let _daemon = env.spawn_daemon(&mut command).await;
    let mut client = env.client().await;

    let mut create = CreateSession::new(
        &repo,
        "Create a file named smoke.txt in the current directory containing the word ok. Do nothing else.",
    );
    create.preset = Some("edits".into());
    let id = client.create(create).await;
    let view = client
        .until(&id, "running", |view| view.holder_pid.is_some())
        .await;
    assert!(!view.unsandboxed);
    let written = view.worktree.join("smoke.txt");

    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_within(Duration::from_secs(60), "Claude to start", |pane| {
        pane.text().contains("trust this folder") || written.exists()
    })
    .await;
    if !written.exists() {
        pane.type_line("").await;
    }
    pane.wait_for_within(Duration::from_secs(180), "smoke.txt", |_| written.exists())
        .await;

    let mut stopper = HolderClient::connect(&env.holder_socket(&id))
        .await
        .unwrap();
    stopper.attach().await.unwrap();
    stopper.send(&ToHolder::Kill).await.unwrap();
    let stopped = client
        .until(&id, "Claude to stop", |view| {
            matches!(view.agent, Some(State::Exited | State::Errored))
        })
        .await;
    assert_eq!(stopped.agent, Some(State::Exited), "{:?}", stopped.error);
}
