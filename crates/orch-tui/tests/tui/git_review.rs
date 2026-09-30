use std::path::Path;
use std::process::Command;
use std::sync::Once;

use orch_git::ENV_REDIRECTING_GIT;
use orch_tui::{ReviewTarget, load_review};

static ISOLATE: Once = Once::new();
fn without_user_git_identity() {
    ISOLATE.call_once(|| {
        // SAFETY: every test here calls this before spawning git, and Once serialises the writes.
        unsafe {
            std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
            std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
            for key in ENV_REDIRECTING_GIT {
                std::env::remove_var(key);
            }
        }
    });
}

fn git(dir: &Path, args: &[&str]) -> String {
    let mut git = Command::new("git");
    for key in ENV_REDIRECTING_GIT {
        git.env_remove(key);
    }
    let output = git
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn the_review_covers_commits_uncommitted_and_untracked_files_but_not_base_progress() {
    without_user_git_identity();
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("login.rs"), "let timeout = 50;\n").unwrap();
    std::fs::write(repo.join("readme.md"), "hello\n").unwrap();
    std::fs::write(repo.join(".gitignore"), "target/\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);
    let worktree = dir.path().join("wt");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "orch/work",
            worktree.to_str().unwrap(),
        ],
    );

    std::fs::write(worktree.join("committed.rs"), "fn a() {}\n").unwrap();
    git(&worktree, &["add", "committed.rs"]);
    git(&worktree, &["commit", "-qm", "work"]);
    std::fs::write(worktree.join("login.rs"), "let timeout = config();\n").unwrap();
    std::fs::write(worktree.join("untracked.rs"), "fn b() {}\n").unwrap();
    std::fs::create_dir(worktree.join("target")).unwrap();
    std::fs::write(worktree.join("target/ignored"), "x").unwrap();
    std::fs::write(repo.join("readme.md"), "hello from main\n").unwrap();
    git(&repo, &["commit", "-qam", "main moves on"]);
    let status_before = git(&worktree, &["status", "--porcelain"]);

    let review = load_review(&ReviewTarget {
        repo: repo.clone(),
        worktree: worktree.clone(),
        slug: "wt".into(),
        branch: "orch/work".into(),
        base: "main".into(),
    })
    .unwrap();
    let files = review.files;

    let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, vec!["committed.rs", "login.rs", "untracked.rs"]);
    let login = &files[1];
    assert_eq!((login.added(), login.removed()), (1, 1));
    assert_eq!(git(&worktree, &["status", "--porcelain"]), status_before);
}

#[test]
fn a_missing_base_is_an_error() {
    without_user_git_identity();
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("a"), "a").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-qm", "init"]);

    let target = ReviewTarget {
        repo: dir.path().to_path_buf(),
        worktree: dir.path().to_path_buf(),
        slug: "x".into(),
        branch: "main".into(),
        base: "no-such-branch".into(),
    };
    assert!(load_review(&target).is_err());
}

#[test]
fn a_file_removed_in_the_worktree_is_marked_as_deleted() {
    without_user_git_identity();
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("old.rs"), "fn old() {}\n").unwrap();
    std::fs::write(repo.join("kept.rs"), "fn kept() {}\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);
    let worktree = dir.path().join("wt");
    let path = worktree.to_str().unwrap();
    git(&repo, &["worktree", "add", "-q", "-b", "orch/work", path]);
    std::fs::remove_file(worktree.join("old.rs")).unwrap();
    std::fs::write(worktree.join("kept.rs"), "").unwrap();

    let review = load_review(&ReviewTarget {
        repo: repo.clone(),
        worktree: worktree.clone(),
        slug: "wt".into(),
        branch: "orch/work".into(),
        base: "main".into(),
    })
    .unwrap();

    let marks: Vec<(&str, bool)> = review
        .files
        .iter()
        .map(|file| (file.path.as_str(), file.deleted))
        .collect();
    assert_eq!(marks, vec![("kept.rs", false), ("old.rs", true)]);
}
