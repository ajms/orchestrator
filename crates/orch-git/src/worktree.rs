use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use crate::git::head_ref;
use crate::{Error, Repo, Script, ScriptOutcome};

pub const DEFAULT_BRANCH_PREFIX: &str = "orch/";
const EXCLUDE_ENTRY: &str = "/.orchestrator/";
const FORCE_EVEN_IF_DIRTY_OR_LOCKED: [&str; 2] = ["--force", "--force"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionName {
    pub slug: String,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionWorktree {
    pub path: PathBuf,
    pub name: SessionName,
    pub base: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorktreeEntry {
    pub path: PathBuf,
    pub branch: Option<String>,
}

impl Repo {
    pub fn worktrees_dir(&self) -> PathBuf {
        self.root().join(".orchestrator/worktrees")
    }

    pub fn worktree_path(&self, slug: &str) -> PathBuf {
        self.worktrees_dir().join(slug)
    }

    pub fn session_worktree(&self, name: &SessionName, base: &str) -> SessionWorktree {
        SessionWorktree {
            path: self.worktree_path(&name.slug),
            name: name.clone(),
            base: base.to_string(),
        }
    }

    pub fn unique_name(&self, prefix: &str, slug: &str) -> SessionName {
        let registered: Vec<PathBuf> = self
            .worktree_entries()
            .unwrap_or_default()
            .into_iter()
            .map(|entry| entry.path)
            .collect();
        (1..)
            .map(|n| match n {
                1 => slug.to_string(),
                n => format!("{slug}-{n}"),
            })
            .map(|slug| SessionName {
                branch: format!("{prefix}{slug}"),
                slug,
            })
            .find(|name| {
                let path = self.worktree_path(&name.slug);
                !self.branch_exists(&name.branch) && !path.exists() && !registered.contains(&path)
            })
            .expect("some suffix is free")
    }

    pub fn create_worktree(
        &self,
        name: &SessionName,
        base: &str,
    ) -> Result<SessionWorktree, Error> {
        self.exclude_orchestrator_dir()?;
        let worktree = self.session_worktree(name, base);
        self.git(["worktree", "add", "-q", "--no-track", "-b", &name.branch])
            .arg(&worktree.path)
            .arg(head_ref(base))
            .run()?;
        Ok(worktree)
    }

    pub fn attach_worktree(&self, name: &SessionName) -> Result<PathBuf, Error> {
        self.git(["worktree", "prune"]).run()?;
        let path = self.worktree_path(&name.slug);
        self.git(["worktree", "add", "-q"])
            .arg(&path)
            .arg(&name.branch)
            .run()?;
        Ok(path)
    }

    pub fn repair_worktrees(&self, paths: &[PathBuf]) -> Result<(), Error> {
        if paths.is_empty() {
            return Ok(());
        }
        self.git(["worktree", "repair"]).args(paths).run().map(drop)
    }

    pub fn worktree_exists(&self, path: &Path) -> bool {
        let Ok(path) = path.canonicalize() else {
            return false;
        };
        self.worktree_entries()
            .is_ok_and(|entries| entries.iter().any(|entry| entry.path == path))
    }

    pub fn worktree_branch(&self, path: &Path) -> Result<Option<String>, Error> {
        let Ok(path) = path.canonicalize() else {
            return Ok(None);
        };
        Ok(self
            .worktree_entries()?
            .into_iter()
            .find(|entry| entry.path == path)
            .and_then(|entry| entry.branch))
    }

    pub fn remove_session_worktree(
        &self,
        worktree: &SessionWorktree,
        teardown: Option<&Script>,
    ) -> Result<Option<ScriptOutcome>, Error> {
        let outcome = self.remove_worktree_dir(&worktree.path, teardown)?;
        self.delete_branch(&worktree.name.branch)?;
        Ok(outcome)
    }

    pub(crate) fn remove_worktree_dir(
        &self,
        path: &Path,
        teardown: Option<&Script>,
    ) -> Result<Option<ScriptOutcome>, Error> {
        let inside = path.starts_with(self.worktrees_dir())
            && path != self.worktrees_dir()
            && !path.components().any(|part| part == Component::ParentDir);
        if !inside {
            return Err(Error::NotAnOrchestratorWorktree {
                path: path.to_path_buf(),
            });
        }
        let mut outcome = None;
        if path.is_dir() {
            outcome = teardown.map(|script| script.run(path));
            if self.worktree_exists(path) {
                self.git(["worktree", "remove"])
                    .args(FORCE_EVEN_IF_DIRTY_OR_LOCKED)
                    .arg(path)
                    .run()?;
            } else {
                fs::remove_dir_all(path)
                    .map_err(|error| Error::io(format!("remove {}", path.display()), error))?;
            }
        }
        self.git(["worktree", "prune"]).run()?;
        Ok(outcome)
    }

    pub(crate) fn delete_branch(&self, branch: &str) -> Result<(), Error> {
        if self.branch_exists(branch) {
            self.git(["branch", "-D", branch]).run()?;
        }
        Ok(())
    }

    pub(crate) fn worktree_entries(&self) -> Result<Vec<WorktreeEntry>, Error> {
        let listing = self.git(["worktree", "list", "--porcelain"]).run()?;
        let mut entries = Vec::new();
        for line in listing.lines() {
            if let Some(path) = line.strip_prefix("worktree ") {
                let path = PathBuf::from(path);
                entries.push(WorktreeEntry {
                    path: path.canonicalize().unwrap_or(path),
                    branch: None,
                });
            } else if let Some(branch) = line.strip_prefix("branch refs/heads/")
                && let Some(entry) = entries.last_mut()
            {
                entry.branch = Some(branch.to_string());
            }
        }
        Ok(entries)
    }

    fn exclude_orchestrator_dir(&self) -> Result<(), Error> {
        let exclude = PathBuf::from(
            self.git([
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "info/exclude",
            ])
            .run()?,
        );
        let current = fs::read_to_string(&exclude).unwrap_or_default();
        if current.lines().any(|line| line.trim() == EXCLUDE_ENTRY) {
            return Ok(());
        }
        let separator = if current.is_empty() || current.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        exclude
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| {
                fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&exclude)
            })
            .and_then(|mut file| writeln!(file, "{separator}{EXCLUDE_ENTRY}"))
            .map_err(|error| Error::io(format!("update {}", exclude.display()), error))
    }
}
