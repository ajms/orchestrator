use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::git::git;
use crate::{Error, Repo};

const BRANCH_PREFIX: &str = "worktree-";

pub fn subagent_worktrees_dir(session_worktree: &Path) -> PathBuf {
    let orchestrator = session_worktree
        .parent()
        .and_then(Path::parent)
        .unwrap_or(session_worktree);
    let slug = session_worktree.file_name().unwrap_or_default();
    orchestrator.join("subagents").join(slug)
}

impl Repo {
    pub fn create_subagent_worktree(
        &self,
        session_worktree: &Path,
        name: &str,
    ) -> Result<PathBuf, Error> {
        let single_component = matches!(
            Path::new(name).components().collect::<Vec<_>>()[..],
            [Component::Normal(_)]
        );
        if !single_component {
            return Err(Error::InvalidSubagentWorktreeName { name: name.into() });
        }
        let dir = subagent_worktrees_dir(session_worktree);
        let path = dir.join(name);
        fs::create_dir_all(&dir)
            .map_err(|error| Error::io(format!("create {}", dir.display()), error))?;
        git(
            session_worktree,
            ["worktree", "add", "-q", "--no-track", "-b"],
        )
        .arg(format!("{BRANCH_PREFIX}{name}"))
        .arg(&path)
        .arg("HEAD")
        .run()?;
        Ok(path)
    }

    pub fn remove_subagent_worktree(
        &self,
        session_worktree: &Path,
        path: &Path,
    ) -> Result<(), Error> {
        let dir = resolve(&subagent_worktrees_dir(session_worktree));
        let path = resolve(path);
        let inside = path.parent() == Some(dir.as_path())
            && !path.components().any(|part| part == Component::ParentDir);
        if !inside {
            return Err(Error::NotAnOrchestratorWorktree { path });
        }
        let branch = self.worktree_branch(&path)?;
        self.remove_checkout(&path)?;
        if let Some(branch) = branch.filter(|branch| merged_into(branch, session_worktree)) {
            self.delete_branch(&branch)?;
        }
        if fs::read_dir(&dir).is_ok_and(|mut entries| entries.next().is_none()) {
            let _ = fs::remove_dir(&dir);
        }
        Ok(())
    }

    pub(crate) fn remove_subagent_worktrees(&self, session_worktree: &Path) {
        for path in self.subagent_worktrees(session_worktree) {
            let _ = self.remove_subagent_worktree(session_worktree, &path);
        }
    }

    pub(crate) fn subagent_worktrees(&self, session_worktree: &Path) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = fs::read_dir(subagent_worktrees_dir(session_worktree))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect();
        paths.sort();
        paths
    }
}

fn resolve(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        match (path.parent().map(Path::canonicalize), path.file_name()) {
            (Some(Ok(parent)), Some(name)) => parent.join(name),
            _ => path.to_path_buf(),
        }
    })
}

fn merged_into(branch: &str, session_worktree: &Path) -> bool {
    session_worktree.is_dir()
        && git(
            session_worktree,
            ["merge-base", "--is-ancestor", branch, "HEAD"],
        )
        .succeeds()
}
