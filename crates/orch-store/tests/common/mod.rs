#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use orch_core::{Phase, SessionId};
use orch_git::ENV_REDIRECTING_GIT;
use orch_store::{NewSession, Repo, RepoRoot, SessionRecord, Store};
use tempfile::TempDir;

pub fn git(dir: &Path, args: &[&str]) {
    let mut command = Command::new("git");
    for key in ENV_REDIRECTING_GIT {
        command.env_remove(key);
    }
    let status = command
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

pub struct Fixture {
    pub dir: TempDir,
    pub store: Store,
}

impl Fixture {
    pub fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let store = Store::open(&dir.path().join("state").join("state.db")).unwrap();
        Self { dir, store }
    }

    pub fn db_path(&self) -> PathBuf {
        self.dir.path().join("state").join("state.db")
    }

    pub fn git_repo(&self, name: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        std::fs::create_dir_all(&path).unwrap();
        git(&path, &["init", "-q"]);
        git(&path, &["commit", "-q", "--allow-empty", "-m", "init"]);
        path.canonicalize().unwrap()
    }

    pub fn register(&mut self, name: &str) -> Repo {
        let path = self.git_repo(name);
        let root = RepoRoot::resolve(&path).unwrap();
        self.store.register_repo(&root).unwrap()
    }
}

pub fn new_session(repo: &Repo, slug: &str) -> NewSession {
    NewSession {
        id: SessionId(format!("session-{slug}")),
        repo: repo.id,
        slug: slug.into(),
        branch: format!("orch/{slug}"),
        base: "main".into(),
        worktree: repo.path.join(".orchestrator/worktrees").join(slug),
        phase: Phase::SettingUp,
        preset: "inherit".into(),
    }
}

impl Fixture {
    pub fn session(&mut self, repo: &Repo, slug: &str) -> SessionRecord {
        self.store.create_session(new_session(repo, slug)).unwrap()
    }

    pub fn reopen(&mut self) {
        self.store = Store::open(&self.db_path()).unwrap();
    }
}
