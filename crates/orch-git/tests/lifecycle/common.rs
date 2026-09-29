use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

use orch_git::{Repo, SessionName, SessionWorktree};
use tempfile::TempDir;

static ISOLATE: Once = Once::new();

fn isolate_git() {
    ISOLATE.call_once(|| {
        let vars = [
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_AUTHOR_NAME", "Test"),
            ("GIT_AUTHOR_EMAIL", "test@example.com"),
            ("GIT_COMMITTER_NAME", "Test"),
            ("GIT_COMMITTER_EMAIL", "test@example.com"),
        ];
        for (key, value) in vars {
            // SAFETY: every test calls `isolate_git` before spawning anything, and Once serialises the writes.
            unsafe { std::env::set_var(key, value) };
        }
    });
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_string()
}

pub fn write(dir: &Path, path: &str, content: &str) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

pub fn read(dir: &Path, path: &str) -> String {
    std::fs::read_to_string(dir.join(path)).unwrap()
}

pub fn commit(dir: &Path, path: &str, content: &str, message: &str) -> String {
    write(dir, path, content);
    git(dir, &["add", path]);
    git(dir, &["commit", "-q", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

pub fn rev(dir: &Path, rev: &str) -> String {
    git(dir, &["rev-parse", rev])
}

pub fn name(slug: &str) -> SessionName {
    SessionName {
        slug: slug.into(),
        branch: format!("orch/{slug}"),
    }
}

pub struct Fixture {
    tmp: TempDir,
    pub root: PathBuf,
    pub origin: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        isolate_git();
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        let root = base.join("repo");
        std::fs::create_dir(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        commit(&root, "README.md", "hello\n", "initial");
        Fixture {
            tmp,
            origin: base.join("origin.git"),
            root,
        }
    }

    pub fn with_origin(default_branch: &str) -> Self {
        let fixture = Fixture::new();
        let origin = fixture.origin.to_str().unwrap();
        let mut push = vec!["push", "-q", "origin", "main"];
        if default_branch != "main" {
            git(&fixture.root, &["branch", default_branch]);
            push.push(default_branch);
        }
        git(
            fixture.outside(),
            &["init", "-q", "--bare", "-b", default_branch, origin],
        );
        git(&fixture.root, &["remote", "add", "origin", origin]);
        git(&fixture.root, &push);
        git(&fixture.root, &["remote", "set-head", "origin", "--auto"]);
        fixture
    }

    pub fn outside(&self) -> &Path {
        self.tmp.path()
    }

    pub fn repo(&self) -> Repo {
        Repo::open(&self.root).unwrap()
    }

    pub fn session(&self, slug: &str) -> SessionWorktree {
        self.session_on(slug, "main")
    }

    pub fn session_on(&self, slug: &str, base: &str) -> SessionWorktree {
        self.repo().create_worktree(&name(slug), base).unwrap()
    }
}
