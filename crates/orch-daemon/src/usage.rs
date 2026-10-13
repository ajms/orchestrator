use std::path::PathBuf;

use orch_protocol::{Reply, RepoUsage, RequestError, UsageReport, UsageTotalsView};
use orch_store::UsageTotals;

use crate::state::Daemon;

impl Daemon {
    pub(crate) async fn usage(&self) -> Result<Reply, RequestError> {
        let (per_repo, today) = self
            .store
            .call(|store| {
                Ok::<_, orch_store::StoreError>((
                    store.usage_per_repo()?,
                    store.usage_per_repo_today()?,
                ))
            })
            .await?
            .map_err(|err| RequestError::Internal {
                message: err.to_string(),
            })?;
        let total = per_repo.iter().map(|(_, totals)| *totals).sum();
        Ok(Reply::Usage(UsageReport {
            per_repo: repo_usage(per_repo),
            today: repo_usage(today),
            total: view(total),
            estimated: true,
        }))
    }
}

fn repo_usage(totals: Vec<(PathBuf, UsageTotals)>) -> Vec<RepoUsage> {
    totals
        .into_iter()
        .map(|(repo, totals)| RepoUsage {
            repo,
            totals: view(totals),
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
