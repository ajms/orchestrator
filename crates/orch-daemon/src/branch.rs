use std::path::Path;

use orch_git::{BranchSource, ResolvedBranch};
use orch_protocol::RequestError;
use orch_store::RepoId;

use crate::lifecycle::refused;
use crate::pr::{BranchPr, PrQueryError, open_prs, pr_by_number};
use crate::state::Daemon;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypedAs {
    BranchName,
    PrNumber,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BranchTarget {
    pub(crate) branch: String,
    pub(crate) typed_as: TypedAs,
    pub(crate) pr: Option<BranchPr>,
}

pub(crate) enum OpenPrRefusal {
    LookupFailed(String),
    Several(Vec<u64>),
}

impl OpenPrRefusal {
    pub(crate) fn describe(&self, branch: &str) -> String {
        match self {
            OpenPrRefusal::LookupFailed(err) => {
                format!("looking up the PRs of {branch} failed: {err}")
            }
            OpenPrRefusal::Several(numbers) => {
                let numbers: Vec<String> = numbers.iter().map(|n| format!("#{n}")).collect();
                format!(
                    "{branch} has {} open PRs ({})",
                    numbers.len(),
                    numbers.join(", ")
                )
            }
        }
    }
}

pub(crate) async fn single_open_pr(
    repo: &Path,
    branch: &str,
) -> Result<Option<BranchPr>, OpenPrRefusal> {
    let mut open = match open_prs(repo, branch).await {
        Ok(open) => open,
        Err(PrQueryError::NoGitHub) => return Ok(None),
        Err(PrQueryError::Failed(err)) => return Err(OpenPrRefusal::LookupFailed(err)),
    };
    if open.len() > 1 {
        return Err(OpenPrRefusal::Several(
            open.iter().map(|pr| pr.number).collect(),
        ));
    }
    Ok(open.pop())
}

impl BranchTarget {
    pub(crate) fn open_pr(&self) -> Option<&BranchPr> {
        self.pr.as_ref().filter(|pr| pr.is_open())
    }

    pub(crate) fn base(&self, requested: Option<String>) -> Result<Option<String>, RequestError> {
        match (self.open_pr(), requested) {
            (Some(pr), Some(requested)) if requested != pr.base => Err(refused(format!(
                "PR #{} targets {}, not {requested}",
                pr.number, pr.base
            ))),
            (Some(pr), _) => Ok(Some(pr.base.clone())),
            (None, requested) => Ok(requested),
        }
    }
}

pub(crate) struct BranchRequest<'a> {
    pub(crate) repo: RepoId,
    pub(crate) target: &'a BranchTarget,
    pub(crate) base: &'a str,
    pub(crate) configured_base: Option<&'a str>,
}

impl Daemon {
    pub(crate) async fn branch_target(
        &self,
        repo: &Path,
        field: &str,
    ) -> Result<BranchTarget, RequestError> {
        if let Some(number) = field.strip_prefix('#') {
            let number = number
                .parse()
                .map_err(|_| refused(format!("{field} is not a PR number")))?;
            let pr = pr_by_number(repo, number).await.map_err(refused)?;
            return Ok(BranchTarget {
                branch: pr.head.clone(),
                typed_as: TypedAs::PrNumber,
                pr: Some(pr),
            });
        }
        let pr = single_open_pr(repo, field)
            .await
            .map_err(|refusal| match refusal {
                OpenPrRefusal::Several(_) => refused(format!(
                    "{}; type #<number> to pick one",
                    refusal.describe(field)
                )),
                OpenPrRefusal::LookupFailed(_) => refused(refusal.describe(field)),
            })?;
        Ok(BranchTarget {
            branch: field.into(),
            typed_as: TypedAs::BranchName,
            pr,
        })
    }

    pub(crate) fn resolve_branch(
        &self,
        git: &orch_git::Repo,
        request: BranchRequest,
    ) -> Result<ResolvedBranch, RequestError> {
        let BranchRequest {
            repo,
            target,
            base,
            configured_base,
        } = request;
        let branch = target.branch.as_str();
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
        let resolved = git.resolve_branch(branch).map_err(refused)?;
        if target.typed_as == TypedAs::PrNumber && resolved.source == BranchSource::New {
            return Err(refused(format!(
                "the PR's Branch {branch} no longer exists"
            )));
        }
        Ok(resolved)
    }

    fn branch_owner(&self, repo: RepoId, branch: &str) -> Option<String> {
        self.lock()
            .sessions
            .values()
            .find(|live| live.record.repo == repo && live.record.branch == branch)
            .map(|live| live.record.slug.clone())
    }
}
