use orch_git::ResolvedBranch;
use orch_protocol::RequestError;
use orch_store::RepoId;

use crate::lifecycle::refused;
use crate::state::Daemon;

pub(crate) struct BranchRequest<'a> {
    pub(crate) repo: RepoId,
    pub(crate) branch: &'a str,
    pub(crate) base: &'a str,
    pub(crate) configured_base: Option<&'a str>,
}

impl Daemon {
    pub(crate) fn resolve_branch(
        &self,
        git: &orch_git::Repo,
        request: BranchRequest,
    ) -> Result<ResolvedBranch, RequestError> {
        let BranchRequest {
            repo,
            branch,
            base,
            configured_base,
        } = request;
        if branch == base {
            return Err(refused(format!("{branch} is the Session's Base branch")));
        }
        let reserved = git.reserved_branches(configured_base).map_err(refused)?;
        if reserved.iter().any(|name| name == branch) {
            return Err(refused(format!(
                "{branch} is the Repo's default Base branch"
            )));
        }
        if let Some(owner) = self.branch_owner(repo, branch) {
            return Err(refused(format!(
                "{branch} is the Branch of Session {owner}"
            )));
        }
        git.resolve_branch(branch).map_err(refused)
    }

    fn branch_owner(&self, repo: RepoId, branch: &str) -> Option<String> {
        self.lock()
            .sessions
            .values()
            .find(|live| live.record.repo == repo && live.record.branch == branch)
            .map(|live| live.record.slug.clone())
    }
}
