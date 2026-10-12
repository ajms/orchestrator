use std::path::Path;

use crate::git::{git, head_ref};
use crate::{Error, Repo, SessionWorktree};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewSnapshot {
    pub tree: String,
    pub merge_base: String,
}

impl Repo {
    pub fn review_snapshot(&self, worktree: &SessionWorktree) -> Result<ReviewSnapshot, Error> {
        let base_ref = head_ref(&worktree.base);
        let base_tip = self
            .git(["rev-parse", "--verify", &format!("{base_ref}^{{commit}}")])
            .run()?;
        let snapshot = snapshot_commit(&worktree.path)?;
        Ok(ReviewSnapshot {
            tree: self
                .git(["rev-parse", &format!("{snapshot}^{{tree}}")])
                .run()?,
            merge_base: self.git(["merge-base", &base_tip, &snapshot]).run()?,
        })
    }

    pub fn diff(&self, from: &str, to: &str) -> Result<String, Error> {
        self.git(["diff", "--no-color", "--no-ext-diff", from, to])
            .run()
    }

    pub fn diff_stat(&self, from: &str, to: &str) -> Result<String, Error> {
        self.git(["diff", "--no-color", "--stat=1000", from, to])
            .run()
    }
}

pub(crate) fn snapshot_commit(worktree: &Path) -> Result<String, Error> {
    let scratch =
        tempfile::tempdir().map_err(|error| Error::io("create temporary index", error))?;
    let index = scratch.path().join("index");
    let real_index = git(
        worktree,
        ["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )
    .run()?;
    if Path::new(&real_index).exists() {
        std::fs::copy(&real_index, &index)
            .map_err(|error| Error::io(format!("copy {real_index}"), error))?;
    }
    git(worktree, ["add", "-A"])
        .env("GIT_INDEX_FILE", &index)
        .run()?;
    let tree = git(worktree, ["write-tree"])
        .env("GIT_INDEX_FILE", &index)
        .run()?;
    git(
        worktree,
        ["commit-tree", &tree, "-p", "HEAD", "-m", "orch snapshot"],
    )
    .env("GIT_AUTHOR_NAME", "orch")
    .env("GIT_AUTHOR_EMAIL", "orch@localhost")
    .env("GIT_COMMITTER_NAME", "orch")
    .env("GIT_COMMITTER_EMAIL", "orch@localhost")
    .run()
}
