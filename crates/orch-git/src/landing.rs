use std::fmt;
use std::path::{Path, PathBuf};

use crate::git::{git, head_ref};
use crate::snapshot::snapshot_commit;
use crate::{Error, Repo, SessionWorktree};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Landed {
    pub commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LandingError {
    Conflict { paths: Vec<String> },
    BaseCheckoutDirty { worktree: PathBuf },
    BaseCheckedOutElsewhere { worktree: PathBuf },
    NothingToLand,
    BaseMoved,
    Failed(Error),
}

impl From<Error> for LandingError {
    fn from(error: Error) -> Self {
        LandingError::Failed(error)
    }
}

impl fmt::Display for LandingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LandingError::Conflict { paths } => write!(
                f,
                "Landing conflicts with the Base branch in: {}",
                paths.join(", ")
            ),
            LandingError::BaseCheckoutDirty { worktree } => write!(
                f,
                "the Base branch is checked out with local changes in {}; commit or stash them first",
                worktree.display()
            ),
            LandingError::BaseCheckedOutElsewhere { worktree } => write!(
                f,
                "the Base branch is checked out in {}; Land that Session first or switch it away",
                worktree.display()
            ),
            LandingError::NothingToLand => write!(f, "the Worktree has no changes to land"),
            LandingError::BaseMoved => write!(f, "the Base branch moved during Landing; retry"),
            LandingError::Failed(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for LandingError {}

impl Repo {
    pub fn land_squash(
        &self,
        worktree: &SessionWorktree,
        message: &str,
    ) -> Result<Landed, LandingError> {
        let base_ref = head_ref(&worktree.base);
        let base_tip = self
            .git(["rev-parse", "--verify", &format!("{base_ref}^{{commit}}")])
            .run()?;
        let snapshot = snapshot_commit(&worktree.path)?;
        let merge_base = self.git(["merge-base", &base_tip, &snapshot]).run()?;
        let tree = self.merge_tree(&merge_base, &base_tip, &snapshot)?;
        let base_tree = self
            .git(["rev-parse", &format!("{base_tip}^{{tree}}")])
            .run()?;
        if tree == base_tree {
            return Err(LandingError::NothingToLand);
        }
        let main_checkout = self.main_checkout_to_update(&worktree.base, &base_tip, &tree)?;
        let commit = self
            .git(["commit-tree", &tree, "-p", &base_tip, "-m", message])
            .run()?;
        self.advance_base(&worktree.base, &base_tip, &commit, main_checkout)?;
        Ok(Landed { commit })
    }

    pub(crate) fn advance_base(
        &self,
        base: &str,
        from: &str,
        to: &str,
        main_checkout: bool,
    ) -> Result<(), LandingError> {
        let base_ref = head_ref(base);
        if !self.git(["update-ref", &base_ref, to, from]).succeeds() {
            return Err(LandingError::BaseMoved);
        }
        if main_checkout && let Err(error) = self.git(["read-tree", "-m", "-u", from, to]).run() {
            self.git(["update-ref", &base_ref, from, to]).run()?;
            return Err(error.into());
        }
        Ok(())
    }

    pub fn commit_worktree(
        &self,
        worktree: &SessionWorktree,
        message: &str,
    ) -> Result<Option<String>, Error> {
        git(&worktree.path, ["add", "-A"]).run()?;
        if git(&worktree.path, ["diff", "--cached", "--quiet"]).succeeds() {
            return Ok(None);
        }
        git(&worktree.path, ["commit", "-q", "-m", message]).run()?;
        git(&worktree.path, ["rev-parse", "HEAD"]).run().map(Some)
    }

    pub fn remote_has_branch(&self, branch: &str) -> Result<bool, Error> {
        let listing = self.git(["ls-remote", "--exit-code", "--heads", "origin"]);
        let command = listing.describe();
        let output = listing.arg(head_ref(branch)).output()?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(2) => Ok(false),
            _ => Err(Error::Git {
                command,
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            }),
        }
    }

    pub fn push(&self, branch: &str) -> Result<(), Error> {
        let branch_ref = head_ref(branch);
        self.git(["push", "-q", "-u", "origin"])
            .arg(format!("{branch_ref}:{branch_ref}"))
            .run()
            .map(drop)
    }

    pub(crate) fn main_checkout_to_update(
        &self,
        base: &str,
        base_tip: &str,
        tree: &str,
    ) -> Result<bool, LandingError> {
        let checkout = self
            .worktree_entries()?
            .into_iter()
            .find(|entry| entry.branch.as_deref() == Some(base))
            .map(|entry| entry.path);
        match checkout {
            None => Ok(false),
            Some(worktree) if worktree != self.root() => {
                Err(LandingError::BaseCheckedOutElsewhere { worktree })
            }
            Some(worktree) if update_would_clobber_local_changes(&worktree, base_tip, tree)? => {
                Err(LandingError::BaseCheckoutDirty { worktree })
            }
            Some(_) => Ok(true),
        }
    }

    fn merge_tree(
        &self,
        merge_base: &str,
        base_tip: &str,
        snapshot: &str,
    ) -> Result<String, LandingError> {
        let merge = self.git([
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            &format!("--merge-base={merge_base}"),
            base_tip,
            snapshot,
        ]);
        let command = merge.describe();
        let output = merge.output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut lines = stdout.lines();
        let tree = lines.next().unwrap_or_default().to_string();
        match output.status.code() {
            Some(0) => Ok(tree),
            Some(1) => {
                let mut paths: Vec<String> = lines
                    .take_while(|line| !line.is_empty())
                    .map(String::from)
                    .collect();
                paths.dedup();
                Err(LandingError::Conflict { paths })
            }
            _ => Err(Error::Git {
                command,
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            }
            .into()),
        }
    }
}

fn update_would_clobber_local_changes(
    checkout: &Path,
    from: &str,
    to: &str,
) -> Result<bool, Error> {
    let status = git(checkout, ["status", "--porcelain", "--untracked-files=no"]).run()?;
    Ok(!status.is_empty() || !git(checkout, ["read-tree", "-m", "-u", "-n", from, to]).succeeds())
}
