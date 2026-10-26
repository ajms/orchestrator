use orch_agent::{
    Placeholder, SandboxLayout, SandboxNeeds, SandboxPaths, sandbox_args, sandbox_carve_outs,
};

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
        masked: Vec::new(),
        configured: SandboxPaths::default(),
        host_env: [
            ("PATH", "/usr/bin"),
            ("LANG", "C.UTF-8"),
            ("LC_TIME", "de_DE.UTF-8"),
            ("GH_TOKEN", "gho_leak"),
            ("AWS_SECRET_ACCESS_KEY", "aws_leak"),
            ("SSH_AUTH_SOCK", "/run/agent.sock"),
            ("DISPLAY", ":0"),
            ("AGENT_API_KEY", "own"),
        ]
        .map(|(key, value)| (key.into(), value.into()))
        .into(),
    }
}

fn needs() -> SandboxNeeds {
    SandboxNeeds {
        writable: vec!["/home/dev/.agent".into(), "/home/dev/.agent.json".into()],
        read_only: vec!["/home/dev/.local/share/agent".into()],
        carve_outs: vec!["/home/dev/.agent/hooks".into()],
        env: vec![("AGENT_NO_UPDATE".into(), "1".into())],
        env_prefixes: vec!["AGENT_".into()],
    }
}

fn bwrap_args() -> Vec<String> {
    sandbox_args(&layout(), &needs())
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

fn carved(path: &str) -> [&str; 3] {
    ["--ro-bind", path, path]
}

fn writable(path: &str) -> [&str; 3] {
    ["--bind", path, path]
}

#[test]
fn the_root_is_read_only_with_a_fresh_dev_and_proc() {
    let args = bwrap_args();
    assert_eq!(
        args[..7],
        ["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc"]
    );
}

#[test]
fn nothing_is_mounted_inside_proc_and_no_flag_breaks_a_nested_sandbox() {
    let args = bwrap_args();
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
    let args = bwrap_args();
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
    let args = bwrap_args();
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
    let args = bwrap_args();
    let git_writable = at(&args, &writable(GIT));
    assert!(at(&args, &carved("/home/dev/shop/.git/hooks")) > git_writable);
    assert!(at(&args, &carved("/home/dev/shop/.git/config")) > git_writable);
    assert!(
        at(&args, &carved("/home/dev/.agent/hooks"))
            > at(
                &args,
                &["--bind-try", "/home/dev/.agent", "/home/dev/.agent"]
            )
    );
}

#[test]
fn mounts_go_root_then_home_then_allowlist_then_writable_then_carve_outs_then_masks() {
    let args = bwrap_args();
    let order = [
        at(&args, &["--ro-bind", "/", "/"]),
        at(&args, &["--tmpfs", HOME]),
        at(&args, &read_only("/home/dev/.gitconfig")),
        at(&args, &writable(WORKTREE)),
        at(&args, &carved("/home/dev/.agent/hooks")),
        at(&args, &["--tmpfs", "/mnt"]),
    ];
    assert!(order.is_sorted(), "{order:?} in {args:#?}");
}

#[test]
fn tmp_is_the_sessions_own_dir_and_tmpdir_points_at_it() {
    let args = bwrap_args();
    at(&args, &["--bind", TMP, "/tmp"]);
    at(&args, &["--setenv", "TMPDIR", "/tmp"]);
}

#[test]
fn the_runtime_dir_is_private_and_holds_only_the_holder_socket() {
    let args = bwrap_args();
    assert!(at(&args, &writable(SOCKET)) > at(&args, &["--tmpfs", RUNTIME]));
}

#[test]
fn without_a_runtime_dir_the_holder_socket_is_still_bound_in() {
    let args = sandbox_args(
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
fn the_environment_is_cleared_and_only_harmless_host_variables_and_the_agents_come_back() {
    let args = bwrap_args();
    let cleared = at(&args, &["--clearenv"]);
    assert!(args[..cleared].iter().all(|arg| arg != "--setenv"));
    for (key, value) in [
        ("PATH", "/usr/bin"),
        ("LANG", "C.UTF-8"),
        ("LC_TIME", "de_DE.UTF-8"),
        ("AGENT_API_KEY", "own"),
        ("AGENT_NO_UPDATE", "1"),
    ] {
        assert!(at(&args, &["--setenv", key, value]) > cleared, "{key}");
    }
    for leak in [
        "GH_TOKEN",
        "AWS_SECRET_ACCESS_KEY",
        "SSH_AUTH_SOCK",
        "DISPLAY",
    ] {
        assert!(!args.iter().any(|arg| arg == leak), "{leak}");
    }
}

#[test]
fn the_agent_starts_in_its_worktree() {
    let args = bwrap_args();
    at(&args, &["--chdir", WORKTREE]);
}

#[test]
fn other_homes_are_hidden() {
    let args = bwrap_args();
    assert!(at(&args, &["--tmpfs", "/home/other"]) > at(&args, &writable(WORKTREE)));
}

#[test]
fn paths_bound_inside_a_hidden_dir_come_back_after_it_is_masked() {
    let repo = "/mnt/data/shop";
    let worktree = "/mnt/data/shop/.orchestrator/worktrees/fix";
    let args = sandbox_args(
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
    let hooks_again = last_at(&args, &carved("/mnt/data/shop/.git/hooks"));
    assert!(mask < repo_again && repo_again < worktree_again && worktree_again < hooks_again);
    assert!(
        last_at(
            &args,
            &["--bind-try", "/home/dev/.agent", "/home/dev/.agent"]
        ) < mask
    );
}

#[test]
fn a_missing_carve_out_gets_an_empty_placeholder_of_its_kind() {
    let needs = SandboxNeeds {
        carve_outs: vec![
            "/home/dev/.agent/hooks".into(),
            "/home/dev/.agent/settings.json".into(),
            "/home/dev/.agent/AGENT.md".into(),
        ],
        ..needs()
    };
    assert_eq!(
        sandbox_carve_outs(&layout(), &needs),
        [
            ("/home/dev/.agent/hooks", Placeholder::Dir),
            ("/home/dev/.agent/settings.json", Placeholder::File("{}")),
            ("/home/dev/.agent/AGENT.md", Placeholder::File("")),
            ("/home/dev/shop/.git/hooks", Placeholder::Dir),
            ("/home/dev/shop/.git/config", Placeholder::File("")),
        ]
        .map(|(path, placeholder)| (path.into(), placeholder))
    );
}

fn effective(args: &[String], path: &str) -> (String, String) {
    let mut found = None;
    let mut rest = args;
    while let [flag, tail @ ..] = rest {
        if flag == "--clearenv" {
            rest = tail;
            continue;
        }
        let width = match flag.as_str() {
            "--bind" | "--bind-try" | "--ro-bind" | "--ro-bind-try" | "--setenv" => 2,
            "--tmpfs" | "--dev" | "--proc" | "--unsetenv" | "--chdir" => 1,
            other => panic!("unknown bwrap flag {other}"),
        };
        let target = &tail[width - 1];
        let mounts = flag != "--setenv" && flag != "--unsetenv" && flag != "--chdir";
        if mounts && std::path::Path::new(path).starts_with(target) {
            found = Some((flag.clone(), target.clone()));
        }
        rest = &tail[width..];
    }
    found.unwrap_or_else(|| panic!("nothing mounted over {path} in {args:#?}"))
}

fn args_with(readable: &[&str], writable: &[&str], hidden: &[&str]) -> Vec<String> {
    let paths = |list: &[&str]| list.iter().map(Into::into).collect();
    sandbox_args(
        &SandboxLayout {
            configured: SandboxPaths {
                readable: paths(readable),
                writable: paths(writable),
                hidden: paths(hidden),
            },
            ..layout()
        },
        &needs(),
    )
}

fn mount(flag: &str, target: &str) -> (String, String) {
    (flag.into(), target.into())
}

#[test]
fn configured_paths_open_home_without_overriding_the_sessions_own_mounts() {
    let args = args_with(
        &["/home/dev/.m2"],
        &[
            "/home/dev/.cargo/registry",
            "/home/dev/shop",
            "/home/dev/.agent",
        ],
        &[],
    );
    assert_eq!(
        effective(&args, "/home/dev/.m2/repo"),
        mount("--ro-bind-try", "/home/dev/.m2")
    );
    assert_eq!(
        effective(&args, "/home/dev/.cargo/registry/index"),
        mount("--bind", "/home/dev/.cargo/registry")
    );
    assert_eq!(
        effective(&args, "/home/dev/.cargo/bin"),
        mount("--ro-bind-try", "/home/dev/.cargo")
    );
    assert_eq!(
        effective(&args, "/home/dev/shop/README.md"),
        mount("--ro-bind", REPO)
    );
    assert_eq!(
        effective(&args, "/home/dev/shop/.git/hooks/pre-commit"),
        mount("--ro-bind", "/home/dev/shop/.git/hooks")
    );
    assert_eq!(
        effective(&args, "/home/dev/.agent/hooks/x"),
        mount("--ro-bind", "/home/dev/.agent/hooks")
    );
}

#[test]
fn hidden_beats_readable_writable_and_the_shipped_toolchains() {
    let args = args_with(
        &["/home/dev/.m2"],
        &["/home/dev/.cargo/registry"],
        &["/home/dev/.cargo", "/home/dev/.m2/secret", "/srv/data"],
    );
    for (path, mask) in [
        ("/home/dev/.cargo/bin", "/home/dev/.cargo"),
        ("/home/dev/.cargo/registry/index", "/home/dev/.cargo"),
        ("/home/dev/.m2/secret/key", "/home/dev/.m2/secret"),
        ("/srv/data/x", "/srv/data"),
    ] {
        assert_eq!(effective(&args, path), mount("--tmpfs", mask), "{path}");
    }
    assert_eq!(
        effective(&args, "/home/dev/.m2/repo"),
        mount("--ro-bind-try", "/home/dev/.m2")
    );
}

#[test]
fn the_agents_own_paths_beat_hidden() {
    let args = args_with(
        &[],
        &[],
        &[
            "/home/dev/.agent",
            "/home/dev/.agent/projects",
            "/home/dev/.local",
        ],
    );
    assert_eq!(
        effective(&args, "/home/dev/.agent/projects/x"),
        mount("--bind-try", "/home/dev/.agent")
    );
    assert_eq!(
        effective(&args, "/home/dev/.agent/hooks/x"),
        mount("--ro-bind", "/home/dev/.agent/hooks")
    );
    assert_eq!(
        effective(&args, "/home/dev/.local/share/agent/bin"),
        mount("--ro-bind-try", "/home/dev/.local/share/agent")
    );
    assert_eq!(
        effective(&args, "/home/dev/.local/share/uv"),
        mount("--tmpfs", "/home/dev/.local")
    );
}

#[test]
fn a_nested_configured_path_wins_over_its_parent_but_never_over_the_agents_own() {
    let args = args_with(
        &["/home/dev/x/sub", "/home/dev/.agent/projects"],
        &["/home/dev/x", "/home/dev/shop/node_modules"],
        &[],
    );
    assert_eq!(
        effective(&args, "/home/dev/x/sub/f"),
        mount("--ro-bind-try", "/home/dev/x/sub")
    );
    assert_eq!(
        effective(&args, "/home/dev/x/f"),
        mount("--bind", "/home/dev/x")
    );
    assert_eq!(
        effective(&args, "/home/dev/shop/node_modules/pkg"),
        mount("--bind", "/home/dev/shop/node_modules")
    );
    assert_eq!(
        effective(&args, "/home/dev/.agent/projects/p"),
        mount("--bind-try", "/home/dev/.agent")
    );
}

#[test]
fn a_masked_file_reads_as_empty_even_inside_a_readable_path() {
    let key = "/home/dev/.config/git/orch-key";
    let args = sandbox_args(
        &SandboxLayout {
            masked: vec![key.into()],
            configured: SandboxPaths {
                readable: vec!["/home/dev/.config".into()],
                ..SandboxPaths::default()
            },
            ..layout()
        },
        &needs(),
    );
    assert_eq!(effective(&args, key), mount("--ro-bind", key));
    let masked = at(&args, &["--ro-bind", "/dev/null", key]);
    assert_eq!(last_at(&args, &["--ro-bind"]), masked);
}
