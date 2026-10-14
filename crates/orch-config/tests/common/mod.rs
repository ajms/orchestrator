#![allow(dead_code)]

use std::path::{Path, PathBuf};

use orch_config::{ConfigLoader, REPO_FILE, RepoConfig};
use tempfile::TempDir;

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
        std::fs::create_dir(dir.path().join("config")).unwrap();
        let loader = ConfigLoader::new(dir.path().join("config/config.toml"))
            .with_home(dir.path().join("home"));
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

    pub fn home(&self, rest: &str) -> PathBuf {
        self.dir.path().join("home").join(rest)
    }

    pub fn repo_path(&self) -> &Path {
        &self.repo
    }
}
