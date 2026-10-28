use crate::git::head_ref;
use crate::worktree::{Upstream, tracking_ref};
use crate::{Error, Repo, SessionName, SessionWorktree};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchSource {
    New,
    Local,
    Origin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBranch {
    pub source: BranchSource,
    fast_forward: Option<(String, String)>,
}

impl Repo {
    pub fn reserved_branches(&self, configured_base: Option<&str>) -> Result<Vec<String>, Error> {
        let mut reserved: Vec<String> = configured_base.map(String::from).into_iter().collect();
        if self.has_origin() {
            reserved.extend(self.origin_head());
        } else if reserved.is_empty() {
            reserved.extend(self.head_branch()?);
        }
        Ok(reserved)
    }

    pub fn resolve_branch(&self, branch: &str) -> Result<ResolvedBranch, Error> {
        self.refuse_checked_out(branch)?;
        let resolved = |source| ResolvedBranch {
            source,
            fast_forward: None,
        };
        match self.compare_with_origin(branch)? {
            Upstream::Absent if self.branch_exists(branch) => Ok(resolved(BranchSource::Local)),
            Upstream::Absent => Ok(resolved(BranchSource::New)),
            Upstream::LocalMissing => Ok(resolved(BranchSource::Origin)),
            Upstream::Same | Upstream::Ahead => Ok(resolved(BranchSource::Local)),
            Upstream::Behind { local, upstream } => Ok(ResolvedBranch {
                source: BranchSource::Local,
                fast_forward: Some((local, upstream)),
            }),
            Upstream::Diverged => Err(Error::BranchDivergedFromOrigin {
                branch: branch.into(),
            }),
        }
    }

    pub fn attach_existing_branch(
        &self,
        name: &SessionName,
        base: &str,
        resolved: &ResolvedBranch,
    ) -> Result<SessionWorktree, Error> {
        let branch = &name.branch;
        self.exclude_orchestrator_dir()?;
        self.fast_forward_from_origin(base)?;
        let worktree = SessionWorktree {
            existing_branch: true,
            ..self.session_worktree(name, base)
        };
        let add = self.git(["worktree", "add", "-q"]);
        let add = match resolved.source {
            BranchSource::Origin => add
                .args(["--track", "-b", branch])
                .arg(&worktree.path)
                .arg(tracking_ref(branch)),
            BranchSource::New | BranchSource::Local => add.arg(&worktree.path).arg(branch),
        };
        if let Some((local, upstream)) = &resolved.fast_forward {
            self.git(["update-ref", &head_ref(branch), upstream, local])
                .run()?;
        }
        add.run()?;
        Ok(worktree)
    }

    fn refuse_checked_out(&self, branch: &str) -> Result<(), Error> {
        let checkout = self
            .worktree_entries()?
            .into_iter()
            .find(|entry| entry.branch.as_deref() == Some(branch));
        match checkout {
            None => Ok(()),
            Some(entry) if entry.path == self.root() => Err(Error::BranchInMainCheckout {
                branch: branch.into(),
            }),
            Some(entry) => Err(Error::BranchCheckedOut {
                branch: branch.into(),
                worktree: entry.path,
            }),
        }
    }
}
