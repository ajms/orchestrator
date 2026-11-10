use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::git::git;
use crate::snapshot::snapshot_commit;
use crate::{Error, Repo};

const BRANCH_PREFIX: &str = "worktree-";

/// The commits bounding a Subagent's work, and the branch it ran on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentRefs {
    /// The Session commit the Subagent worktree was created from.
    pub base: String,
    /// The Subagent's work: a live snapshot, or the tip recorded at removal.
    pub tip: String,
    /// The Subagent's branch, `worktree-<name>`. It may since have been
    /// deleted (after merging back) or preserved by the Agent.
    pub branch: String,
}

/// The branch a Subagent worktree named `name` is created on.
pub fn subagent_branch(name: &str) -> String {
    format!("{BRANCH_PREFIX}{name}")
}

/// The directory holding a Session's Subagent worktrees.
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
        validate_name(name)?;
        let dir = subagent_worktrees_dir(session_worktree);
        let path = dir.join(name);
        fs::create_dir_all(&dir)
            .map_err(|error| Error::io(format!("create {}", dir.display()), error))?;
        git(
            session_worktree,
            ["worktree", "add", "-q", "--no-track", "-b"],
        )
        .arg(subagent_branch(name))
        .arg(&path)
        .arg("HEAD")
        .run()?;
        let base = git(&path, ["rev-parse", "--verify", "HEAD^{commit}"]).run()?;
        git(session_worktree, ["update-ref"])
            .arg(subagent_ref(session_worktree, name, "base"))
            .arg(&base)
            .run()?;
        Ok(path)
    }

    /// Remove a Subagent worktree, first recording its tip so a Subagent
    /// Review still works afterwards.
    ///
    /// Removal proceeds even when the tip cannot be recorded; that failure is
    /// returned as `Ok(Some(error))` for the caller to report.
    pub fn remove_subagent_worktree(
        &self,
        session_worktree: &Path,
        path: &Path,
    ) -> Result<Option<Error>, Error> {
        let dir = resolve(&subagent_worktrees_dir(session_worktree));
        let path = resolve(path);
        let inside = path.parent() == Some(dir.as_path())
            && !path.components().any(|part| part == Component::ParentDir);
        if !inside {
            return Err(Error::NotAnOrchestratorWorktree { path });
        }
        let branch = self.worktree_branch(&path)?;
        let tip_error = path.file_name().and_then(|name| {
            let tip = subagent_ref(session_worktree, &name.to_string_lossy(), "tip");
            snapshot_commit(&path)
                .and_then(|snapshot| {
                    git(session_worktree, ["update-ref"])
                        .arg(&tip)
                        .arg(&snapshot)
                        .run()
                })
                .err()
        });
        self.remove_checkout(&path)?;
        if let Some(branch) = branch.filter(|branch| merged_into(branch, session_worktree)) {
            self.delete_branch(&branch)?;
        }
        if fs::read_dir(&dir).is_ok_and(|mut entries| entries.next().is_none()) {
            let _ = fs::remove_dir(&dir);
        }
        Ok(tip_error)
    }

    pub(crate) fn remove_subagent_worktrees(&self, session_worktree: &Path) {
        for path in self.subagent_worktrees(session_worktree) {
            let _ = self.remove_subagent_worktree(session_worktree, &path);
        }
        let prefix = subagent_refs_prefix(session_worktree);
        if let Ok(refs) = self
            .git(["for-each-ref", "--format=%(refname)"])
            .arg(&prefix)
            .run()
        {
            for name in refs.lines().filter(|line| !line.is_empty()) {
                let _ = self.git(["update-ref", "-d"]).arg(name).run();
            }
        }
    }

    /// The commits bounding a Subagent's work, and its branch, if known.
    ///
    /// A live Subagent worktree is snapshotted now (including uncommitted and
    /// untracked files) for the tip; a removed one uses the tip recorded at
    /// removal. The refs are written inside the Sandbox, so they are
    /// untrusted: they are ignored unless both resolve to commits, the base is
    /// in the Session's history and the tip descends from the base.
    pub fn subagent_diff_refs(
        &self,
        session_worktree: &Path,
        name: &str,
    ) -> Result<Option<SubagentRefs>, Error> {
        validate_name(name)?;
        let commit = |reference: &str| {
            self.git(["rev-parse", "--verify", "--quiet"])
                .arg(format!("{reference}^{{commit}}"))
                .run()
                .ok()
                .filter(|oid| !oid.is_empty())
        };
        let Some(base) = commit(&subagent_ref(session_worktree, name, "base")) else {
            return Ok(None);
        };
        let live = subagent_worktrees_dir(session_worktree).join(name);
        let tip = if self.worktree_exists(&live) {
            Some(snapshot_commit(&live)?)
        } else {
            commit(&subagent_ref(session_worktree, name, "tip"))
        };
        let Some(tip) = tip else {
            return Ok(None);
        };
        let Some(session_head) = session_worktree
            .is_dir()
            .then(|| {
                git(session_worktree, ["rev-parse", "--verify", "HEAD^{commit}"])
                    .run()
                    .ok()
            })
            .flatten()
        else {
            return Ok(None);
        };
        let ancestor =
            |a: &str, b: &str| self.git(["merge-base", "--is-ancestor", a, b]).succeeds();
        if !ancestor(&base, &session_head) || !ancestor(&base, &tip) {
            return Ok(None);
        }
        Ok(Some(SubagentRefs {
            base,
            tip,
            branch: subagent_branch(name),
        }))
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

fn validate_name(name: &str) -> Result<(), Error> {
    let single_component = matches!(
        Path::new(name).components().collect::<Vec<_>>()[..],
        [Component::Normal(_)]
    );
    let valid_ref = single_component
        && !name.starts_with('.')
        && !name.ends_with(".lock")
        && !name.contains("..")
        && !name.contains("@{")
        && !name
            .chars()
            .any(|c| c.is_control() || " ~^:?*[\\".contains(c));
    if !valid_ref {
        return Err(Error::InvalidSubagentWorktreeName { name: name.into() });
    }
    Ok(())
}

fn subagent_refs_prefix(session_worktree: &Path) -> String {
    let slug = session_worktree.file_name().unwrap_or_default();
    format!("refs/orch/subagents/{}/", slug.to_string_lossy())
}

fn subagent_ref(session_worktree: &Path, name: &str, which: &str) -> String {
    format!("{}{name}/{which}", subagent_refs_prefix(session_worktree))
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
