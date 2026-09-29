use std::fs;
use std::path::PathBuf;

use crate::{Error, Repo, Script, ScriptOutcome, SessionName};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Leftover {
    Worktree(PathBuf),
    Branch(String),
}

impl Repo {
    pub fn leftovers(&self, prefix: &str, known: &[SessionName]) -> Result<Vec<Leftover>, Error> {
        let known_paths: Vec<PathBuf> = known
            .iter()
            .map(|name| self.worktree_path(&name.slug))
            .collect();
        let mut worktrees: Vec<PathBuf> = fs::read_dir(self.worktrees_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .filter(|path| !known_paths.contains(path))
            .collect();
        worktrees.sort();
        let listing = self
            .git([
                "for-each-ref",
                "--format=%(refname:lstrip=2)",
                "refs/heads/",
            ])
            .run()?;
        let branches = listing
            .lines()
            .filter(|branch| branch.starts_with(prefix))
            .filter(|branch| !known.iter().any(|name| name.branch == *branch))
            .map(|branch| Leftover::Branch(branch.to_string()));
        Ok(worktrees
            .into_iter()
            .map(Leftover::Worktree)
            .chain(branches)
            .collect())
    }

    pub fn remove_leftover(
        &self,
        leftover: &Leftover,
        teardown: Option<&Script>,
    ) -> Result<Option<ScriptOutcome>, Error> {
        match leftover {
            Leftover::Worktree(path) => self.remove_worktree_dir(path, teardown),
            Leftover::Branch(branch) => self.delete_branch(branch).map(|()| None),
        }
    }
}
