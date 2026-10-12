#![allow(dead_code)]

use std::path::{Path, PathBuf};

use orch_agent::{AgentAdapter, ClaudeCode};
use orch_config::{ConfigLoader, REPO_FILE, RepoConfig};
use orch_core::PermissionMode;
use tempfile::TempDir;

pub fn claude() -> &'static [PermissionMode] {
    ClaudeCode::default().modes()
}

pub struct Fixture {
    pub dir: TempDir,
    pub loader: ConfigLoader,
    pub repo: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        let repo = repo.canonicalize().unwrap();
        let loader = ConfigLoader::new(dir.path().join("config.toml"));
        Self { dir, loader, repo }
    }

    pub fn global(&self, text: &str) {
        std::fs::write(self.loader.global_path(), text).unwrap();
    }

    pub fn repo_file(&self, text: &str) {
        std::fs::write(self.repo.join(REPO_FILE), text).unwrap();
    }

    pub fn personal(&self, body: &str) -> String {
        format!("[repos.{}]\n{body}", self.repo_key())
    }

    pub fn repo_key(&self) -> String {
        format!("{:?}", self.repo.display().to_string())
    }

    pub fn untrusted(&self) -> RepoConfig {
        self.loader.repo(&self.repo, None).unwrap()
    }

    pub fn approved(&self) -> RepoConfig {
        let untrusted = self.untrusted();
        let hash = untrusted.trust_request().map(|request| request.hash);
        self.loader.repo(&self.repo, hash.as_ref()).unwrap()
    }

    pub fn repo_path(&self) -> &Path {
        &self.repo
    }
}
