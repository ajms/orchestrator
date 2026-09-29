use std::path::{Path, PathBuf};

use orch_agent::{GuardContext, GuardDecision, GuardHit, GuardKind, evaluate_guard};
use serde_json::json;

const REPO: &str = "/home/dev/shop";
const WORKTREE: &str = "/home/dev/shop/.orchestrator/worktrees/fix-login";

fn context(worktree: &Path) -> GuardContext<'_> {
    GuardContext {
        worktree,
        branch: "orch/fix-login",
        base_branch: "main",
        enabled: true,
        allowed: &[],
    }
}

fn decide(tool: &str, input: serde_json::Value) -> GuardDecision {
    evaluate_guard(
        tool,
        &input.to_string(),
        None,
        &context(Path::new(WORKTREE)),
    )
}

fn bash(command: &str) -> GuardDecision {
    decide("Bash", json!({ "command": command, "description": "test" }))
}

fn ask(kind: GuardKind, target: &str) -> GuardDecision {
    GuardDecision::Ask(GuardHit {
        kind,
        target: target.into(),
    })
}

fn assert_allowed(commands: &[&str]) {
    for command in commands {
        assert_eq!(bash(command), GuardDecision::Allow, "{command}");
    }
}

fn assert_asks(cases: &[(&str, GuardKind, &str)]) {
    for (command, kind, target) in cases {
        assert_eq!(bash(command), ask(*kind, target), "{command}");
    }
}

#[test]
fn tools_that_stay_inside_the_session_are_allowed() {
    assert_eq!(
        decide("Read", json!({ "file_path": "/etc/passwd" })),
        GuardDecision::Allow
    );
    assert_eq!(
        decide("Grep", json!({ "pattern": "x", "path": "/" })),
        GuardDecision::Allow
    );
    assert_allowed(&[
        "cargo test",
        "ls -la /etc",
        "cat /etc/hosts | grep local",
        "git status && git diff main...HEAD",
        "git log --oneline main..",
    ]);
}

#[test]
fn file_edits_inside_the_worktree_are_allowed() {
    for (tool, key) in [
        ("Write", "file_path"),
        ("Edit", "file_path"),
        ("MultiEdit", "file_path"),
        ("NotebookEdit", "notebook_path"),
    ] {
        for path in [
            format!("{WORKTREE}/src/login.rs"),
            "src/login.rs".to_string(),
        ] {
            assert_eq!(
                decide(tool, json!({ key: path })),
                GuardDecision::Allow,
                "{tool} {path}"
            );
        }
    }
}

#[test]
fn file_edits_outside_the_worktree_ask_with_the_resolved_path() {
    let main_checkout_file = format!("{REPO}/src/login.rs");
    for (tool, key, path) in [
        ("Write", "file_path", main_checkout_file.clone()),
        (
            "Edit",
            "file_path",
            format!("{WORKTREE}/../../../src/login.rs"),
        ),
        (
            "NotebookEdit",
            "notebook_path",
            "../../../src/login.rs".to_string(),
        ),
    ] {
        assert_eq!(
            decide(tool, json!({ key: path })),
            ask(GuardKind::WriteOutsideWorktree, &main_checkout_file),
            "{tool} {path}"
        );
    }
}

#[test]
fn guards_that_are_off_allow_everything() {
    let worktree = Path::new(WORKTREE);
    let off = GuardContext {
        enabled: false,
        ..context(worktree)
    };
    for command in [
        "git push origin HEAD:main",
        "git worktree add ../x",
        "touch /etc/x",
    ] {
        let input = json!({ "command": command }).to_string();
        assert_eq!(
            evaluate_guard("Bash", &input, None, &off),
            GuardDecision::Allow,
            "{command}"
        );
    }
}

#[test]
fn an_allowance_for_the_session_covers_only_that_kind_and_target() {
    let worktree = Path::new(WORKTREE);
    let allowed = [
        GuardHit {
            kind: GuardKind::WorktreeManagement,
            target: "prune".into(),
        },
        GuardHit {
            kind: GuardKind::OtherRef,
            target: "spike".into(),
        },
    ];
    let with_allowances = GuardContext {
        allowed: &allowed,
        ..context(worktree)
    };
    let check = |command: &str| {
        evaluate_guard(
            "Bash",
            &json!({ "command": command }).to_string(),
            None,
            &with_allowances,
        )
    };
    assert_eq!(check("git worktree prune"), GuardDecision::Allow);
    assert_eq!(check("git branch -D spike"), GuardDecision::Allow);
    assert_eq!(
        check("git worktree add ../x"),
        ask(GuardKind::WorktreeManagement, "add")
    );
    assert_eq!(check("git branch other"), ask(GuardKind::OtherRef, "other"));
}

#[test]
fn relative_paths_resolve_against_the_agents_reported_cwd() {
    let worktree = Path::new(WORKTREE);
    let crate_dir = format!("{WORKTREE}/crates/app");
    let check = |tool: &str, input: serde_json::Value, cwd: &str| {
        evaluate_guard(
            tool,
            &input.to_string(),
            Some(Path::new(cwd)),
            &context(worktree),
        )
    };
    assert_eq!(
        check("Write", json!({ "file_path": "src/lib.rs" }), &crate_dir),
        GuardDecision::Allow
    );
    assert_eq!(
        check("Bash", json!({ "command": "touch notes.md" }), REPO),
        ask(GuardKind::WriteOutsideWorktree, "/home/dev/shop/notes.md")
    );
    assert_eq!(
        check(
            "Bash",
            json!({ "command": "echo x > ../../../../../x" }),
            &crate_dir
        ),
        ask(GuardKind::WriteOutsideWorktree, "/home/dev/shop/x")
    );
}

#[test]
fn merging_prs_and_mutating_github_api_calls_ask() {
    assert_allowed(&[
        "gh pr view 12",
        "gh pr create --fill",
        "gh api repos/acme/shop/pulls",
        "gh api -X GET repos/acme/shop/pulls",
    ]);
    assert_asks(&[
        (
            "gh pr merge 12 --squash",
            GuardKind::OtherRef,
            "gh pr merge 12",
        ),
        ("gh pr merge", GuardKind::OtherRef, "gh pr merge"),
        (
            "gh api -X DELETE repos/acme/shop/git/refs/heads/main",
            GuardKind::OtherRef,
            "gh api repos/acme/shop/git/refs/heads/main",
        ),
        (
            "gh api --method=PATCH repos/acme/shop/git/refs/heads/main -f sha=abc",
            GuardKind::OtherRef,
            "gh api repos/acme/shop/git/refs/heads/main",
        ),
        (
            "gh api repos/acme/shop/merges -f base=main -f head=orch/fix-login",
            GuardKind::OtherRef,
            "gh api repos/acme/shop/merges",
        ),
    ]);
}

#[test]
fn pushing_the_sessions_own_branch_is_allowed() {
    assert_allowed(&[
        "git push",
        "git push -u origin HEAD",
        "git push origin orch/fix-login",
        "git push --force-with-lease origin +orch/fix-login",
        "git push origin HEAD:refs/heads/orch/fix-login",
        "git push origin --delete orch/fix-login",
        "git push --dry-run origin HEAD:main",
        "git commit -am 'wip' && git push",
    ]);
}

#[test]
fn pushing_to_the_base_branch_or_other_refs_asks() {
    assert_asks(&[
        ("git push origin HEAD:main", GuardKind::BaseBranch, "main"),
        ("git push origin main", GuardKind::BaseBranch, "main"),
        (
            "git push -f origin +HEAD:refs/heads/main",
            GuardKind::BaseBranch,
            "main",
        ),
        ("git push origin :release", GuardKind::OtherRef, "release"),
        (
            "git push origin --delete release",
            GuardKind::OtherRef,
            "release",
        ),
        (
            "git push origin feature/other",
            GuardKind::OtherRef,
            "feature/other",
        ),
        ("git push --tags", GuardKind::OtherRef, "tags"),
        ("git push --all origin", GuardKind::OtherRef, "all branches"),
        (
            "git push origin v1.2.0:refs/tags/v1.2.0",
            GuardKind::OtherRef,
            "refs/tags/v1.2.0",
        ),
    ]);
}

#[test]
fn listing_branches_and_tags_is_allowed_but_changing_them_asks() {
    assert_allowed(&[
        "git branch",
        "git branch -a -v",
        "git branch --show-current",
        "git branch --list 'orch/*'",
        "git branch --contains HEAD",
        "git tag",
        "git tag -l 'v*'",
    ]);
    assert_asks(&[
        ("git branch spike", GuardKind::OtherRef, "spike"),
        ("git branch -D main", GuardKind::BaseBranch, "main"),
        ("git branch -f main HEAD", GuardKind::BaseBranch, "main"),
        (
            "git branch -m orch/renamed",
            GuardKind::OtherRef,
            "orch/renamed",
        ),
        ("git tag v1.2.0", GuardKind::OtherRef, "v1.2.0"),
        ("git tag -d v1.2.0", GuardKind::OtherRef, "v1.2.0"),
    ]);
}

#[test]
fn switching_the_worktree_away_from_its_branch_asks() {
    assert_allowed(&[
        "git checkout -- src/login.rs",
        "git checkout main -- src/login.rs",
        "git checkout orch/fix-login",
        "git switch orch/fix-login",
    ]);
    assert_asks(&[
        ("git checkout main", GuardKind::BaseBranch, "main"),
        ("git checkout -b spike", GuardKind::OtherRef, "spike"),
        ("git switch main", GuardKind::BaseBranch, "main"),
        (
            "git switch -c spike origin/main",
            GuardKind::OtherRef,
            "spike",
        ),
        ("git switch --detach HEAD~2", GuardKind::OtherRef, "HEAD~2"),
    ]);
}

#[test]
fn history_work_on_the_sessions_own_branch_is_allowed() {
    assert_allowed(&[
        "git add -A && git commit -m 'Fix login'",
        "git rebase main",
        "git merge --no-ff main",
        "git reset --hard HEAD~1",
        "git fetch origin",
        "git pull --rebase origin main",
        "git stash && git stash pop",
        "git update-ref refs/heads/orch/fix-login HEAD",
    ]);
}

#[test]
fn moving_other_refs_directly_asks() {
    assert_asks(&[
        (
            "git update-ref refs/heads/main HEAD",
            GuardKind::BaseBranch,
            "main",
        ),
        (
            "git update-ref -d refs/heads/spike",
            GuardKind::OtherRef,
            "spike",
        ),
        ("git fetch origin main:main", GuardKind::BaseBranch, "main"),
        (
            "git symbolic-ref HEAD refs/heads/main",
            GuardKind::BaseBranch,
            "main",
        ),
    ]);
}

#[test]
fn managing_worktrees_asks_but_listing_them_is_allowed() {
    assert_allowed(&["git worktree list", "git worktree list --porcelain"]);
    assert_asks(&[
        (
            "git worktree add ../spike -b spike",
            GuardKind::WorktreeManagement,
            "add",
        ),
        (
            "git worktree remove --force .",
            GuardKind::WorktreeManagement,
            "remove",
        ),
        (
            "git -C /home/dev/shop worktree prune",
            GuardKind::WorktreeManagement,
            "prune",
        ),
    ]);
}

#[test]
fn shell_writes_outside_the_worktree_ask_best_effort() {
    assert_allowed(&[
        "cargo build > build.log 2>&1",
        "cargo test 2>/dev/null",
        "mkdir -p target/tmp && touch target/tmp/x",
        "echo done | tee -a notes.md",
        "cp README.md docs/",
        "rm -rf target",
    ]);
    assert_asks(&[
        (
            "echo x > /etc/motd",
            GuardKind::WriteOutsideWorktree,
            "/etc/motd",
        ),
        (
            "echo x >> ../../../notes.md",
            GuardKind::WriteOutsideWorktree,
            "/home/dev/shop/notes.md",
        ),
        (
            "cargo build &> /var/log/build.log",
            GuardKind::WriteOutsideWorktree,
            "/var/log/build.log",
        ),
        (
            "echo x | tee /home/dev/shared.txt",
            GuardKind::WriteOutsideWorktree,
            "/home/dev/shared.txt",
        ),
        (
            "cp src/login.rs /home/dev/shop/src/",
            GuardKind::WriteOutsideWorktree,
            "/home/dev/shop/src",
        ),
        (
            "mv ../../../Cargo.toml Cargo.toml",
            GuardKind::WriteOutsideWorktree,
            "/home/dev/shop/Cargo.toml",
        ),
        (
            "rm -rf /home/dev/shop/target",
            GuardKind::WriteOutsideWorktree,
            "/home/dev/shop/target",
        ),
        (
            "cd /home/dev/shop && touch x",
            GuardKind::WriteOutsideWorktree,
            "/home/dev/shop/x",
        ),
        (
            "git -C /home/dev/shop commit -m sneaky",
            GuardKind::WriteOutsideWorktree,
            "/home/dev/shop",
        ),
    ]);
}

#[test]
fn writing_temp_files_is_allowed() {
    let temp = std::env::temp_dir();
    for path in [
        "/tmp/notes.md".to_string(),
        "/tmp/claude-1000/project/session/scratchpad/plan.md".to_string(),
        temp.join("scratch.txt").to_string_lossy().into_owned(),
    ] {
        assert_eq!(
            decide("Write", json!({ "file_path": path })),
            GuardDecision::Allow,
            "{path}"
        );
    }
    assert_allowed(&[
        "cargo test > /tmp/test.log 2>&1",
        "mkdir -p /tmp/claude-1000/x && touch /tmp/claude-1000/x/y",
    ]);
    assert_asks(&[(
        "echo x > /tmp/../etc/motd",
        GuardKind::WriteOutsideWorktree,
        "/etc/motd",
    )]);
}

#[test]
fn quoted_text_is_not_mistaken_for_commands_or_redirections() {
    assert_allowed(&[
        r#"git commit -m "fix > redirect; git push origin main""#,
        "echo 'git worktree add ../x' > notes.txt",
        r#"grep -n "a|b" src/*.rs"#,
    ]);
}

#[test]
fn unknowable_bash_targets_are_left_to_the_agents_own_permissions() {
    assert_allowed(&[
        "echo x > \"$OUT\"",
        "touch $(mktemp)",
        "git push origin \"$BRANCH\"",
    ]);
}

#[test]
fn unparsable_tool_input_is_not_guarded() {
    let decision = evaluate_guard("Write", "not json", None, &context(Path::new(WORKTREE)));
    assert_eq!(decision, GuardDecision::Allow);
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("orch-guards-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("repo/.orchestrator/worktrees/wt")).unwrap();
        std::fs::create_dir_all(root.join("elsewhere")).unwrap();
        Self(root)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn symlinks_are_resolved_before_judging_a_write() {
    let scratch = Scratch::new("symlinks");
    let worktree = scratch.0.join("repo/.orchestrator/worktrees/wt");
    std::os::unix::fs::symlink(scratch.0.join("elsewhere"), worktree.join("escape")).unwrap();
    std::os::unix::fs::symlink(&worktree, scratch.0.join("wt-link")).unwrap();

    let write = |path: PathBuf| {
        let input = json!({ "file_path": path }).to_string();
        evaluate_guard("Write", &input, None, &context(&worktree))
    };
    let escaped = scratch
        .0
        .join("elsewhere/new.txt")
        .canonicalize()
        .unwrap_or_else(|_| scratch.0.canonicalize().unwrap().join("elsewhere/new.txt"));
    assert_eq!(
        write(worktree.join("escape/new.txt")),
        ask(GuardKind::WriteOutsideWorktree, escaped.to_str().unwrap())
    );
    assert_eq!(
        write(scratch.0.join("wt-link/src/new.rs")),
        GuardDecision::Allow
    );
}
