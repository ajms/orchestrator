use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoRoot {
    path: PathBuf,
    head_branch: Option<String>,
}

impl RepoRoot {
    pub fn resolve(inside: &Path) -> Result<Self, orch_git::Error> {
        let repo = orch_git::Repo::open(inside)?;
        Ok(Self {
            head_branch: repo.head_branch()?,
            path: repo.root().to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn head_branch(&self) -> Option<&str> {
        self.head_branch.as_deref()
    }
}
