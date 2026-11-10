use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::git::git;
use crate::snapshot::snapshot_commit;
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
        validate_name(name)?;
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
        let base = git(&path, ["rev-parse", "--verify", "HEAD^{commit}"]).run()?;
        git(session_worktree, ["update-ref"])
            .arg(subagent_ref(session_worktree, name, "base"))
            .arg(&base)
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
        if let (Some(name), Ok(snapshot)) = (path.file_name(), snapshot_commit(&path)) {
            let _ = git(session_worktree, ["update-ref"])
                .arg(subagent_ref(
                    session_worktree,
                    &name.to_string_lossy(),
                    "head",
                ))
                .arg(&snapshot)
                .run();
        }
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

    /// The `(base, head)` commits bounding a subagent's work, if known.
    ///
    /// A live subagent worktree is snapshotted now (including uncommitted and
    /// untracked files); a removed one uses the snapshot taken at removal.
    /// The refs are written inside the Sandbox, so they are untrusted: they
    /// are ignored unless both resolve to commits, the base is in the
    /// Session's history and the head descends from the base.
    pub fn subagent_diff_refs(
        &self,
        session_worktree: &Path,
        name: &str,
    ) -> Result<Option<(String, String)>, Error> {
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
        let head = if self.worktree_exists(&live) {
            Some(snapshot_commit(&live)?)
        } else {
            commit(&subagent_ref(session_worktree, name, "head"))
        };
        let Some(head) = head else {
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
        if !ancestor(&base, &session_head) || !ancestor(&base, &head) {
            return Ok(None);
        }
        Ok(Some((base, head)))
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
