use std::path::Path;

use crate::cleanup::files_under;
use crate::git::{git, head_ref};
use crate::{Error, Leftover, Repo, SessionName, SessionWorktree};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub id: String,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscardPreview {
    pub uncommitted: Vec<String>,
    pub unlanded: Vec<Commit>,
}

impl Repo {
    pub fn discard_preview(&self, worktree: &SessionWorktree) -> Result<DiscardPreview, Error> {
        let uncommitted = if worktree.path.is_dir() {
            uncommitted_files(&worktree.path)?
        } else {
            Vec::new()
        };
        Ok(DiscardPreview {
            uncommitted,
            unlanded: self.unlanded_commits(&worktree.name.branch, &worktree.base)?,
        })
    }

    pub fn leftover_preview(
        &self,
        leftover: &Leftover,
        base: &str,
    ) -> Result<DiscardPreview, Error> {
        match leftover {
            Leftover::Worktree(path) => match self.worktree_branch(path)? {
                Some(branch) => self.discard_preview(&SessionWorktree {
                    path: path.clone(),
                    name: SessionName {
                        slug: String::new(),
                        branch,
                    },
                    base: base.into(),
                }),
                None => Ok(DiscardPreview {
                    uncommitted: files_under(path),
                    unlanded: Vec::new(),
                }),
            },
            Leftover::Branch(branch) => Ok(DiscardPreview {
                uncommitted: Vec::new(),
                unlanded: self.unlanded_commits(branch, base)?,
            }),
        }
    }

    fn unlanded_commits(&self, branch: &str, base: &str) -> Result<Vec<Commit>, Error> {
        let branch_ref = head_ref(branch);
        let exclusion = if self.branch_exists(base) {
            vec![head_ref(base)]
        } else {
            vec![format!("--exclude={branch}"), "--branches".into()]
        };
        let log = self
            .git(["log", "--format=%H %s", &branch_ref, "--not"])
            .args(exclusion)
            .run()?;
        Ok(log
            .lines()
            .filter_map(|line| line.split_once(' '))
            .map(|(id, subject)| Commit {
                id: id.into(),
                subject: subject.into(),
            })
            .collect())
    }
}

fn uncommitted_files(worktree: &Path) -> Result<Vec<String>, Error> {
    let status = git(
        worktree,
        ["status", "--porcelain", "-z", "--untracked-files=all"],
    )
    .run()?;
    let mut entries = status.split('\0').filter(|entry| !entry.is_empty());
    let mut paths = Vec::new();
    while let Some(entry) = entries.next() {
        let (code, path) = entry.split_at(3.min(entry.len()));
        if code.contains('R') || code.contains('C') {
            entries.next();
        }
        paths.push(path.to_string());
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}
