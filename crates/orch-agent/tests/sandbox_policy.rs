use orch_agent::{SandboxLayout, SandboxNeeds, sandbox_policy};

const HOME: &str = "/home/dev";
const REPO: &str = "/home/dev/shop";
const WORKTREE: &str = "/home/dev/shop/.orchestrator/worktrees/fix";
const SUBAGENTS: &str = "/home/dev/shop/.orchestrator/subagents/fix";
const GIT: &str = "/home/dev/shop/.git";
const TMP: &str = "/home/dev/.local/state/orchestrator/sessions/s1/tmp";
const RUNTIME: &str = "/run/user/1000";
const SOCKET: &str = "/run/user/1000/orchestrator/holders/s1.sock";

fn layout() -> SandboxLayout {
    SandboxLayout {
        home: HOME.into(),
        tmp: TMP.into(),
        runtime_dir: Some(RUNTIME.into()),
        holder_socket: SOCKET.into(),
        worktree: WORKTREE.into(),
        subagent_worktrees: SUBAGENTS.into(),
        main_checkout: REPO.into(),
        git_common_dir: GIT.into(),
        read_only: vec!["/home/dev/bin/orch".into()],
        hidden: vec!["/home/other".into(), "/mnt".into()],
    }
}

fn needs() -> SandboxNeeds {
    SandboxNeeds {
        writable: vec!["/home/dev/.agent".into(), "/home/dev/.agent.json".into()],
        read_only: vec!["/home/dev/.local/share/agent".into()],
        carve_outs: vec!["/home/dev/.agent/hooks".into()],
        env: vec![("AGENT_NO_UPDATE".into(), "1".into())],
    }
}

fn policy() -> Vec<String> {
    sandbox_policy(&layout(), &needs())
}

fn find(args: &[String], words: &[&str]) -> Option<usize> {
    args.windows(words.len())
        .position(|window| window.iter().zip(words).all(|(arg, word)| arg == word))
}

fn at(args: &[String], words: &[&str]) -> usize {
    find(args, words).unwrap_or_else(|| panic!("{words:?} missing from {args:#?}"))
}

fn last_at(args: &[String], words: &[&str]) -> usize {
    args.windows(words.len())
        .rposition(|window| window.iter().zip(words).all(|(arg, word)| arg == word))
        .unwrap_or_else(|| panic!("{words:?} missing from {args:#?}"))
}

fn read_only(path: &str) -> [&str; 3] {
    ["--ro-bind-try", path, path]
}

fn writable(path: &str) -> [&str; 3] {
    ["--bind", path, path]
}

#[test]
fn the_root_is_read_only_with_a_fresh_dev_and_proc() {
    let args = policy();
    assert_eq!(
        args[..7],
        ["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc"]
    );
}

#[test]
fn nothing_is_mounted_inside_proc_and_no_flag_breaks_a_nested_sandbox() {
    let args = policy();
    assert!(
        !args
            .iter()
            .any(|arg| arg.starts_with("/proc/") || arg == "/proc/")
    );
    for flag in [
        "--new-session",
        "--unshare-pid",
        "--unshare-all",
        "--disable-userns",
        "--die-with-parent",
    ] {
        assert!(!args.contains(&flag.to_string()), "{flag}");
    }
}

#[test]
fn home_is_a_private_tmpfs_with_only_the_allowlist_bound_back_read_only() {
    let args = policy();
    at(&args, &["--tmpfs", HOME]);
    for path in [
        "/home/dev/.gitconfig",
        "/home/dev/.config/git",
        "/home/dev/.cargo",
        "/home/dev/.rustup",
        "/home/dev/.local/share/uv",
        "/home/dev/.nvm",
        "/home/dev/.pyenv",
        "/home/dev/.local/share/agent",
        "/home/dev/bin/orch",
    ] {
        at(&args, &read_only(path));
    }
    for secret in [".ssh", ".config/gh", ".gnupg", ".aws", ".netrc"] {
        assert!(!args.iter().any(|arg| arg.contains(secret)), "{secret}");
    }
}

#[test]
fn the_worktree_its_subagent_worktrees_and_the_git_common_dir_are_writable() {
    let args = policy();
    at(&args, &["--ro-bind", REPO, REPO]);
    for path in [WORKTREE, SUBAGENTS, GIT] {
        assert!(at(&args, &writable(path)) > at(&args, &["--ro-bind", REPO, REPO]));
    }
    at(
        &args,
        &["--bind-try", "/home/dev/.agent", "/home/dev/.agent"],
    );
    at(
        &args,
        &[
            "--bind-try",
            "/home/dev/.agent.json",
            "/home/dev/.agent.json",
        ],
    );
}

#[test]
fn git_hooks_and_config_and_the_agents_carve_outs_stay_read_only() {
    let args = policy();
    let git_writable = at(&args, &writable(GIT));
    assert!(at(&args, &read_only("/home/dev/shop/.git/hooks")) > git_writable);
    assert!(at(&args, &read_only("/home/dev/shop/.git/config")) > git_writable);
    assert!(
        at(&args, &read_only("/home/dev/.agent/hooks"))
            > at(
                &args,
                &["--bind-try", "/home/dev/.agent", "/home/dev/.agent"]
            )
    );
}

#[test]
fn mounts_go_root_then_home_then_allowlist_then_writable_then_carve_outs_then_masks() {
    let args = policy();
    let order = [
        at(&args, &["--ro-bind", "/", "/"]),
        at(&args, &["--tmpfs", HOME]),
        at(&args, &read_only("/home/dev/.gitconfig")),
        at(&args, &writable(WORKTREE)),
        at(&args, &read_only("/home/dev/.agent/hooks")),
        at(&args, &["--tmpfs", "/mnt"]),
    ];
    assert!(order.is_sorted(), "{order:?} in {args:#?}");
}

#[test]
fn tmp_is_the_sessions_own_dir_and_tmpdir_points_at_it() {
    let args = policy();
    at(&args, &["--bind", TMP, "/tmp"]);
    at(&args, &["--setenv", "TMPDIR", "/tmp"]);
}

#[test]
fn the_runtime_dir_is_private_and_holds_only_the_holder_socket() {
    let args = policy();
    assert!(at(&args, &writable(SOCKET)) > at(&args, &["--tmpfs", RUNTIME]));
}

#[test]
fn without_a_runtime_dir_the_holder_socket_is_still_bound_in() {
    let args = sandbox_policy(
        &SandboxLayout {
            runtime_dir: None,
            ..layout()
        },
        &needs(),
    );
    at(&args, &writable(SOCKET));
    assert!(find(&args, &["--tmpfs", RUNTIME]).is_none());
}

#[test]
fn agent_forwarding_and_desktop_sessions_are_unset_and_the_agents_env_is_set() {
    let args = policy();
    for var in [
        "SSH_AUTH_SOCK",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "DBUS_SESSION_BUS_ADDRESS",
    ] {
        at(&args, &["--unsetenv", var]);
    }
    at(&args, &["--setenv", "AGENT_NO_UPDATE", "1"]);
}

#[test]
fn the_agent_starts_in_its_worktree() {
    let args = policy();
    at(&args, &["--chdir", WORKTREE]);
}

#[test]
fn other_homes_are_hidden() {
    let args = policy();
    assert!(at(&args, &["--tmpfs", "/home/other"]) > at(&args, &writable(WORKTREE)));
}

#[test]
fn paths_bound_inside_a_hidden_dir_come_back_after_it_is_masked() {
    let repo = "/mnt/data/shop";
    let worktree = "/mnt/data/shop/.orchestrator/worktrees/fix";
    let args = sandbox_policy(
        &SandboxLayout {
            main_checkout: repo.into(),
            worktree: worktree.into(),
            subagent_worktrees: "/mnt/data/shop/.orchestrator/subagents/fix".into(),
            git_common_dir: "/mnt/data/shop/.git".into(),
            ..layout()
        },
        &needs(),
    );
    let mask = at(&args, &["--tmpfs", "/mnt"]);
    let repo_again = last_at(&args, &["--ro-bind", repo, repo]);
    let worktree_again = last_at(&args, &writable(worktree));
    let hooks_again = last_at(&args, &read_only("/mnt/data/shop/.git/hooks"));
    assert!(mask < repo_again && repo_again < worktree_again && worktree_again < hooks_again);
    assert!(
        last_at(
            &args,
            &["--bind-try", "/home/dev/.agent", "/home/dev/.agent"]
        ) < mask
    );
}
