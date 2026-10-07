use std::collections::BTreeMap;

use orch_protocol::{AgentTotals, Reply, RepoUsage, RequestError, UsageReport, UsageTotalsView};
use orch_store::{RepoAgentUsage, UsageTotals};

use crate::state::Daemon;

impl Daemon {
    pub(crate) async fn usage(&self) -> Result<Reply, RequestError> {
        let (per_repo, today) = self
            .store
            .call(|store| {
                Ok::<_, orch_store::StoreError>((
                    store.usage_per_repo_and_agent()?,
                    store.usage_per_repo_and_agent_today()?,
                ))
            })
            .await?
            .map_err(|err| RequestError::Internal {
                message: err.to_string(),
            })?;
        Ok(Reply::Usage(UsageReport {
            per_agent: per_agent(&per_repo),
            per_repo: repo_usage(per_repo),
            today: repo_usage(today),
            estimated: true,
        }))
    }
}

fn per_agent(per_repo: &[RepoAgentUsage]) -> Vec<AgentTotals> {
    let mut totals = BTreeMap::<&str, UsageTotals>::new();
    for usage in per_repo {
        totals
            .entry(&usage.agent)
            .and_modify(|total| *total = total.plus(usage.totals))
            .or_insert(usage.totals);
    }
    totals
        .into_iter()
        .map(|(agent, totals)| AgentTotals {
            agent: agent.into(),
            totals: view(totals),
        })
        .collect()
}

fn repo_usage(totals: Vec<RepoAgentUsage>) -> Vec<RepoUsage> {
    totals
        .into_iter()
        .map(|usage| RepoUsage {
            repo: usage.repo,
            agent: usage.agent,
            totals: view(usage.totals),
        })
        .collect()
}

fn view(totals: UsageTotals) -> UsageTotalsView {
    UsageTotalsView {
        input_tokens: totals.input_tokens,
        output_tokens: totals.output_tokens,
        cost_usd: totals.cost_usd,
    }
}
