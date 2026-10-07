use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::git::git;
use crate::{Error, Repo};

const BRANCH_PREFIX: &str = "worktree-";
const FORCE_EVEN_IF_DIRTY_OR_LOCKED: [&str; 2] = ["--force", "--force"];

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
        let dir = subagent_worktrees_dir(session_worktree);
        let path = dir.join(name);
        let single_component = matches!(
            Path::new(name).components().collect::<Vec<_>>()[..],
            [Component::Normal(_)]
        );
        if !single_component {
            return Err(Error::NotAnOrchestratorWorktree { path });
        }
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
        let dir = subagent_worktrees_dir(session_worktree);
        let inside = path.parent() == Some(dir.as_path())
            && !path.components().any(|part| part == Component::ParentDir);
        if !inside {
            return Err(Error::NotAnOrchestratorWorktree {
                path: path.to_path_buf(),
            });
        }
        let branch = self.worktree_branch(path)?;
        if self.worktree_exists(path) {
            self.git(["worktree", "remove"])
                .args(FORCE_EVEN_IF_DIRTY_OR_LOCKED)
                .arg(path)
                .run()?;
        } else if path.exists() {
            fs::remove_dir_all(path)
                .map_err(|error| Error::io(format!("remove {}", path.display()), error))?;
        }
        if let Some(branch) = branch.filter(|branch| merged_into(branch, session_worktree)) {
            self.delete_branch(&branch)?;
        }
        if fs::read_dir(&dir).is_ok_and(|mut entries| entries.next().is_none()) {
            let _ = fs::remove_dir(&dir);
        }
        Ok(())
    }

    pub(crate) fn remove_subagent_worktrees(&self, session_worktree: &Path) -> Result<(), Error> {
        let dir = subagent_worktrees_dir(session_worktree);
        let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .collect();
        paths.sort();
        for path in paths {
            self.remove_subagent_worktree(session_worktree, &path)?;
        }
        Ok(())
    }
}

fn merged_into(branch: &str, session_worktree: &Path) -> bool {
    session_worktree.is_dir()
        && git(
            session_worktree,
            ["merge-base", "--is-ancestor", branch, "HEAD"],
        )
        .succeeds()
}
