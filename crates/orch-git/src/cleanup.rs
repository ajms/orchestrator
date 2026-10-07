use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;

use crate::git::head_ref;
use crate::{Error, Repo, Script, ScriptOutcome, SessionWorktree};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupCheckpoint {
    pub worktree: Option<u64>,
    pub branch_tip: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InUse {
    pub worktree: bool,
    pub branch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleaned {
    pub kept_worktree: bool,
    pub kept_branch: bool,
    pub teardown: Option<ScriptOutcome>,
}

impl Repo {
    pub fn cleanup_checkpoint(&self, worktree: &SessionWorktree) -> CleanupCheckpoint {
        CleanupCheckpoint {
            worktree: digest(&worktree.path),
            branch_tip: self.branch_tip(&worktree.name.branch),
        }
    }

    pub fn finish_session_cleanup(
        &self,
        worktree: &SessionWorktree,
        since: &CleanupCheckpoint,
        in_use: InUse,
        teardown: Option<&Script>,
    ) -> Result<Cleaned, Error> {
        let path = &worktree.path;
        let branch = &worktree.name.branch;
        let present = path.exists();
        let checked_out = self.worktree_branch(path)?;
        let worktree_ours = present
            && !in_use.worktree
            && since.worktree.is_some()
            && digest(path) == since.worktree
            && checked_out.as_ref().is_none_or(|current| current == branch);
        let mut outcome = None;
        if worktree_ours {
            outcome = teardown.map(|script| script.run(path));
            self.remove_worktree_dir(path, None)?;
        } else if !present {
            self.remove_subagent_worktrees(path);
            self.git(["worktree", "prune"]).run()?;
        }
        let kept_worktree = present && !worktree_ours;
        let branch_held = kept_worktree && checked_out.as_ref() == Some(branch);
        let branch_ours = !in_use.branch
            && !branch_held
            && since.branch_tip.is_some()
            && self.branch_tip(branch) == since.branch_tip;
        if branch_ours {
            self.delete_branch(branch)?;
        }
        Ok(Cleaned {
            kept_worktree,
            kept_branch: !branch_ours && self.branch_exists(branch),
            teardown: outcome,
        })
    }

    fn branch_tip(&self, branch: &str) -> Option<String> {
        self.git(["rev-parse", "--verify", "--quiet", &head_ref(branch)])
            .run()
            .ok()
    }
}

fn digest(dir: &Path) -> Option<u64> {
    if !dir.is_dir() {
        return None;
    }
    let mut entries = Vec::new();
    collect(dir, dir, &mut entries);
    entries.sort();
    let mut hasher = DefaultHasher::new();
    entries.hash(&mut hasher);
    Some(hasher.finish())
}

fn collect(root: &Path, dir: &Path, entries: &mut Vec<(String, u64, i128)>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            collect(root, &path, entries);
            continue;
        }
        let modified = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_nanos() as i128);
        let relative = path.strip_prefix(root).unwrap_or(&path);
        entries.push((
            relative.to_string_lossy().into_owned(),
            meta.len(),
            modified,
        ));
    }
}

pub(crate) fn files_under(dir: &Path) -> Vec<String> {
    let mut entries = Vec::new();
    collect(dir, dir, &mut entries);
    let mut files: Vec<String> = entries.into_iter().map(|(path, _, _)| path).collect();
    files.sort();
    files
}
