use std::path::Path;
use std::sync::Arc;

use orch_git::{BranchSource, ResolvedBranch};
use orch_protocol::{
    BranchFound, BranchLookup, LookupBranch, LookupDepth, OpenPr, Reply, RequestError,
};
use orch_store::RepoRoot;

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
    fn named(branch: String) -> Self {
        Self {
            branch,
            typed_as: TypedAs::BranchName,
            pr: None,
        }
    }

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
    pub(crate) repo: &'a Path,
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
            pr,
            ..BranchTarget::named(field.into())
        })
    }

    pub(crate) async fn lookup_branch(self: &Arc<Self>, lookup: LookupBranch) -> Reply {
        let outcome =
            self.look_up_branch(lookup)
                .await
                .unwrap_or_else(|err| BranchLookup::Refused {
                    reason: err.to_string(),
                });
        Reply::BranchLookup(outcome)
    }

    async fn look_up_branch(
        self: &Arc<Self>,
        lookup: LookupBranch,
    ) -> Result<BranchLookup, RequestError> {
        let LookupBranch {
            repo,
            branch,
            base,
            depth,
        } = lookup;
        let root = tokio::task::spawn_blocking(move || RepoRoot::resolve(&repo))
            .await
            .map_err(refused)?
            .map_err(refused)?;
        let root = root.path().to_path_buf();
        let config = self.repo_config(&root).await?;
        let _guard = match depth {
            LookupDepth::Origin => Some(self.repo_guard(&root).await),
            LookupDepth::Local => None,
        };
        let target = match depth {
            LookupDepth::Origin => self.branch_target(&root, &branch).await?,
            LookupDepth::Local if branch.starts_with('#') => {
                return Ok(BranchLookup::NotRefusedLocally);
            }
            LookupDepth::Local => BranchTarget::named(branch),
        };
        let requested = target.base(base)?;
        let daemon = self.clone();
        tokio::task::spawn_blocking(move || {
            let git = orch_git::Repo::open(&root).map_err(refused)?;
            let configured_base = config.base_branch();
            let base = match requested {
                Some(base) => base,
                None => git.known_default_base(configured_base).map_err(refused)?,
            };
            let request = BranchRequest {
                repo: &root,
                target: &target,
                base: &base,
                configured_base,
            };
            if depth == LookupDepth::Local {
                daemon.check_local_branch(&git, request)?;
                return Ok(BranchLookup::NotRefusedLocally);
            }
            let found = match daemon.resolve_branch(&git, request)?.source {
                BranchSource::New => return Ok(BranchLookup::New { base }),
                BranchSource::Local => BranchFound::Local,
                BranchSource::Origin => BranchFound::Origin,
            };
            Ok(BranchLookup::Existing {
                pr: target.open_pr().map(|pr| OpenPr {
                    number: pr.number,
                    title: pr.title.clone(),
                    base: pr.base.clone(),
                }),
                branch: target.branch,
                found,
            })
        })
        .await
        .map_err(refused)?
    }

    pub(crate) fn resolve_branch(
        &self,
        git: &orch_git::Repo,
        request: BranchRequest,
    ) -> Result<ResolvedBranch, RequestError> {
        let reserved = git
            .reserved_branches(request.configured_base)
            .map_err(refused)?;
        self.refuse_name(&request, &reserved)?;
        let target = request.target;
        let branch = target.branch.as_str();
        let resolved = git.resolve_branch(branch).map_err(refused)?;
        if target.typed_as == TypedAs::PrNumber && resolved.source == BranchSource::New {
            return Err(refused(format!(
                "the PR's Branch {branch} no longer exists"
            )));
        }
        Ok(resolved)
    }

    fn check_local_branch(
        &self,
        git: &orch_git::Repo,
        request: BranchRequest,
    ) -> Result<(), RequestError> {
        let reserved = git
            .known_reserved_branches(request.configured_base)
            .map_err(refused)?;
        self.refuse_name(&request, &reserved)?;
        git.refuse_checked_out(&request.target.branch)
            .map_err(refused)
    }

    fn refuse_name(
        &self,
        request: &BranchRequest,
        reserved: &[String],
    ) -> Result<(), RequestError> {
        let &BranchRequest {
            repo, target, base, ..
        } = request;
        let branch = target.branch.as_str();
        if branch == base {
            return Err(refused(format!("{branch} is the Session's Base branch")));
        }
        if reserved.iter().any(|name| name == branch) {
            return Err(refused(format!(
                "{branch} is the Repo's default Base branch"
            )));
        }
        match self.branch_owner(repo, branch) {
            Some(owner) => Err(refused(format!(
                "{branch} is the Branch of Session {owner}"
            ))),
            None => Ok(()),
        }
    }

    fn branch_owner(&self, repo: &Path, branch: &str) -> Option<String> {
        self.lock()
            .sessions
            .values()
            .find(|live| live.repo == repo && live.record.branch == branch)
            .map(|live| live.record.slug.clone())
    }
}
