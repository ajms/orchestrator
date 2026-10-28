use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use crate::git::head_ref;
use crate::{Error, Repo, Script, ScriptOutcome, slugify};

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
    pub existing_branch: bool,
}

pub(crate) enum Upstream {
    Absent,
    LocalMissing,
    Same,
    Ahead,
    Behind { local: String, upstream: String },
    Diverged,
}

pub(crate) fn tracking_ref(branch: &str) -> String {
    format!("refs/remotes/origin/{branch}")
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
            existing_branch: false,
        }
    }

    pub fn unique_name(&self, prefix: &str, slug: &str) -> SessionName {
        self.free_name(prefix, slug, None)
    }

    pub fn name_for_branch(&self, prefix: &str, branch: &str) -> SessionName {
        let wanted = branch.strip_prefix(prefix).unwrap_or(branch);
        SessionName {
            slug: self.free_name(prefix, &slugify(wanted), Some(branch)).slug,
            branch: branch.to_string(),
        }
    }

    fn free_name(&self, prefix: &str, slug: &str, own_branch: Option<&str>) -> SessionName {
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
                let branch_free =
                    own_branch == Some(name.branch.as_str()) || !self.branch_exists(&name.branch);
                branch_free && !path.exists() && !registered.contains(&path)
            })
            .expect("some suffix is free")
    }

    pub fn create_worktree(
        &self,
        name: &SessionName,
        base: &str,
    ) -> Result<SessionWorktree, Error> {
        self.exclude_orchestrator_dir()?;
        self.fast_forward_from_origin(base)?;
        let worktree = self.session_worktree(name, base);
        self.git(["worktree", "add", "-q", "--no-track", "-b", &name.branch])
            .arg(&worktree.path)
            .arg(head_ref(base))
            .run()?;
        Ok(worktree)
    }

    pub(crate) fn fast_forward_from_origin(&self, base: &str) -> Result<(), Error> {
        match self.compare_with_origin(base)? {
            Upstream::Absent | Upstream::LocalMissing | Upstream::Same => Ok(()),
            Upstream::Ahead => Err(Error::BaseAheadOfOrigin { base: base.into() }),
            Upstream::Diverged => Err(Error::BaseDivergedFromOrigin { base: base.into() }),
            Upstream::Behind { local, upstream } => self
                .main_checkout_to_update(base, &local, &upstream)
                .and_then(|main_checkout| self.advance_base(base, &local, &upstream, main_checkout))
                .map_err(|landing| Error::BaseNotFastForwarded {
                    base: base.into(),
                    reason: landing.to_string(),
                }),
        }
    }

    pub(crate) fn has_origin(&self) -> bool {
        self.git(["remote", "get-url", "origin"]).succeeds()
    }

    pub(crate) fn compare_with_origin(&self, branch: &str) -> Result<Upstream, Error> {
        if !self.has_origin() {
            return Ok(Upstream::Absent);
        }
        let on_origin = self
            .remote_has_branch(branch)
            .map_err(|_| Error::OriginUnreachable)?;
        if !on_origin {
            return Ok(Upstream::Absent);
        }
        let tracking = tracking_ref(branch);
        self.git(["fetch", "-q", "--no-tags", "origin"])
            .arg(format!("+{}:{tracking}", head_ref(branch)))
            .run()?;
        if !self.branch_exists(branch) {
            return Ok(Upstream::LocalMissing);
        }
        let local = self
            .git(["rev-parse", "--verify", "--quiet", &head_ref(branch)])
            .run()?;
        let upstream = self
            .git(["rev-parse", "--verify", "--quiet", &tracking])
            .run()?;
        if local == upstream {
            return Ok(Upstream::Same);
        }
        let is_ancestor = |from: &str, to: &str| {
            self.git(["merge-base", "--is-ancestor", from, to])
                .succeeds()
        };
        Ok(if is_ancestor(&upstream, &local) {
            Upstream::Ahead
        } else if is_ancestor(&local, &upstream) {
            Upstream::Behind { local, upstream }
        } else {
            Upstream::Diverged
        })
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
        let subagents = paths.iter().flat_map(|path| self.subagent_worktrees(path));
        self.git(["worktree", "repair"])
            .args(paths)
            .args(subagents)
            .run()
            .map(drop)
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
        if !worktree.existing_branch {
            self.delete_branch(&worktree.name.branch)?;
        }
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
        self.remove_subagent_worktrees(path);
        if path.is_dir() {
            outcome = teardown.map(|script| script.run(path));
            self.remove_checkout(path)?;
        }
        self.git(["worktree", "prune"]).run()?;
        Ok(outcome)
    }

    pub(crate) fn remove_checkout(&self, path: &Path) -> Result<(), Error> {
        if self.worktree_exists(path) {
            self.git(["worktree", "remove"])
                .args(FORCE_EVEN_IF_DIRTY_OR_LOCKED)
                .arg(path)
                .run()?;
        } else if path.exists() {
            fs::remove_dir_all(path)
                .map_err(|error| Error::io(format!("remove {}", path.display()), error))?;
        }
        Ok(())
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

    pub(crate) fn exclude_orchestrator_dir(&self) -> Result<(), Error> {
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
