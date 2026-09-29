use crate::git::head_ref;
use crate::{Error, Repo, SessionWorktree};

impl Repo {
    pub fn contains_base_tip(&self, worktree: &SessionWorktree) -> Result<bool, Error> {
        let base_ref = head_ref(&worktree.base);
        self.git(["rev-parse", "--verify", "--quiet", &base_ref])
            .run()?;
        Ok(self
            .git(["merge-base", "--is-ancestor", &base_ref])
            .arg(head_ref(&worktree.name.branch))
            .succeeds())
    }
}
