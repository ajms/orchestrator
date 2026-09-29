use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use orch_config::ConfigLoader;
use orch_core::{ChecksState, PhaseEvent, PrState, PrStatus, ReviewDecision, SessionId};
use orch_protocol::{Reply, RequestError};
use orch_store::SessionRecord;
use serde::Deserialize;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::lifecycle::{Busy, refused, with_git};
use crate::state::{CommentCursor, Daemon, session_worktree};
use crate::subprocess;

const GH_TIMEOUT: Duration = Duration::from_secs(120);
const POLL_TIMEOUT: Duration = Duration::from_secs(30);
const POLLS_AT_ONCE: usize = 4;
const PR_FIELDS: &str = "state,statusCheckRollup,reviewDecision,comments,reviews";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPr {
    state: GhState,
    #[serde(default)]
    status_check_rollup: Option<Vec<GhCheck>>,
    #[serde(default)]
    review_decision: Option<GhReview>,
    #[serde(default)]
    comments: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    reviews: Option<Vec<GhReviewComment>>,
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum GhState {
    Open,
    Closed,
    Merged,
    #[serde(other)]
    Unknown,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum GhReview {
    Approved,
    ChangesRequested,
    ReviewRequired,
    #[serde(other)]
    None,
}

#[derive(Deserialize)]
struct GhReviewComment {
    #[serde(default)]
    body: String,
}

#[derive(Deserialize)]
struct GhCheck {
    #[serde(default)]
    status: Option<GhCheckStatus>,
    #[serde(default)]
    conclusion: Option<GhConclusion>,
    #[serde(default)]
    state: Option<GhContextState>,
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum GhCheckStatus {
    Completed,
    #[serde(other)]
    Running,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum GhConclusion {
    Success,
    Neutral,
    Skipped,
    #[serde(other)]
    Failed,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum GhContextState {
    Success,
    Pending,
    Expected,
    #[serde(other)]
    Failed,
}

impl GhCheck {
    fn checks(&self) -> ChecksState {
        if let Some(state) = &self.state {
            return match state {
                GhContextState::Success => ChecksState::Passing,
                GhContextState::Pending | GhContextState::Expected => ChecksState::Pending,
                GhContextState::Failed => ChecksState::Failing,
            };
        }
        if self.status != Some(GhCheckStatus::Completed) {
            return ChecksState::Pending;
        }
        match self.conclusion {
            Some(GhConclusion::Success | GhConclusion::Neutral | GhConclusion::Skipped) => {
                ChecksState::Passing
            }
            _ => ChecksState::Failing,
        }
    }
}

impl GhPr {
    fn checks(&self) -> ChecksState {
        let states: Vec<ChecksState> = self
            .status_check_rollup
            .iter()
            .flatten()
            .map(GhCheck::checks)
            .collect();
        [
            ChecksState::Failing,
            ChecksState::Pending,
            ChecksState::Passing,
        ]
        .into_iter()
        .find(|wanted| states.contains(wanted))
        .unwrap_or(ChecksState::None)
    }

    fn review(&self) -> ReviewDecision {
        match self.review_decision {
            Some(GhReview::Approved) => ReviewDecision::Approved,
            Some(GhReview::ChangesRequested) => ReviewDecision::ChangesRequested,
            Some(GhReview::ReviewRequired) => ReviewDecision::ReviewRequired,
            Some(GhReview::None) | None => ReviewDecision::None,
        }
    }

    fn comment_count(&self) -> u32 {
        let comments = self.comments.as_ref().map_or(0, Vec::len);
        let reviews = self
            .reviews
            .iter()
            .flatten()
            .filter(|review| !review.body.trim().is_empty())
            .count();
        u32::try_from(comments + reviews).unwrap_or(u32::MAX)
    }
}

enum PushError {
    Git(orch_git::Error),
    Config(String),
    BaseNotOnOrigin(String),
}

impl From<orch_git::Error> for PushError {
    fn from(error: orch_git::Error) -> Self {
        PushError::Git(error)
    }
}

impl fmt::Display for PushError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PushError::Git(error) => write!(f, "pushing failed: {error}"),
            PushError::Config(error) => write!(f, "reading the config failed: {error}"),
            PushError::BaseNotOnOrigin(base) => write!(
                f,
                "the Base branch {base} is not on origin; push it first so the PR has a base"
            ),
        }
    }
}

fn push_for_pr(
    git: &orch_git::Repo,
    loader: &ConfigLoader,
    branch: &str,
    base: &str,
) -> Result<(), PushError> {
    if !git.remote_has_branch(base)? {
        let prefix = loader
            .global()
            .map_err(|err| PushError::Config(err.to_string()))?
            .branch_prefix;
        if !(base.starts_with(&prefix) && git.branch_exists(base)) {
            return Err(PushError::BaseNotOnOrigin(base.into()));
        }
        git.push(base)?;
    }
    git.push(branch)?;
    Ok(())
}

fn pr_number(output: &str) -> Option<u64> {
    let url = output.lines().rev().find(|line| !line.trim().is_empty())?;
    url.trim()
        .trim_end_matches('/')
        .rsplit('/')
        .next()?
        .parse()
        .ok()
}

async fn gh(repo: &Path, args: &[&str], limit: Duration) -> Result<String, String> {
    let mut command = tokio::process::Command::new("gh");
    command.args(args).current_dir(repo);
    let what = format!(
        "gh {}",
        args.iter().take(2).copied().collect::<Vec<_>>().join(" ")
    );
    subprocess::run(command, None, limit, &what).await
}

pub(crate) async fn retarget_pr(repo: &Path, number: u64, base: &str) -> Result<(), String> {
    gh(
        repo,
        &["pr", "edit", &number.to_string(), "--base", base],
        GH_TIMEOUT,
    )
    .await
    .map(drop)
}

impl Daemon {
    pub(crate) async fn open_pr(
        self: &Arc<Self>,
        id: &SessionId,
        record: SessionRecord,
        repo: PathBuf,
        title: String,
        body: String,
    ) -> Result<Reply, RequestError> {
        let worktree = session_worktree(&record);
        let message = title.clone();
        let committed = with_git(repo.clone(), move |git| {
            git.commit_worktree(&worktree, &message)
        })
        .await?
        .map_err(refused)?;
        let explain = |err: String| match &committed {
            Some(commit) => refused(format!(
                "the uncommitted changes were committed as {} \"{title}\" and stay on the Branch, but {err}",
                &commit[..commit.len().min(12)]
            )),
            None => refused(err),
        };
        let loader = self.config.loader.clone();
        let (branch, base) = (record.branch.clone(), record.base.clone());
        with_git(repo.clone(), move |git| {
            push_for_pr(git, &loader, &branch, &base)
        })
        .await?
        .map_err(|err| explain(err.to_string()))?;
        let created = gh(
            &repo,
            &[
                "pr",
                "create",
                "--head",
                &record.branch,
                "--base",
                &record.base,
                "--title",
                &title,
                "--body",
                &body,
            ],
            GH_TIMEOUT,
        )
        .await
        .map_err(explain)?;
        let number = pr_number(&created)
            .ok_or_else(|| explain(format!("gh printed no PR link: {}", created.trim())))?;
        self.update(id, |live| {
            live.comments = CommentCursor::opened();
            live.transition(PhaseEvent::PrOpened { number })
        })?;
        Ok(Reply::PrOpened {
            number,
            committed_changes: committed.is_some(),
        })
    }

    pub(crate) fn abandon_pr(&self, id: &SessionId) -> Result<Reply, RequestError> {
        self.update(id, |live| {
            let closed = live
                .status
                .flags()
                .pr
                .as_ref()
                .is_some_and(|pr| pr.state == PrState::Closed);
            if !closed || live.exclusive {
                return Err(
                    "only a Session whose PR was closed without merging can leave it".into(),
                );
            }
            live.transition(PhaseEvent::PrAbandoned)
        })?;
        Ok(Reply::Done)
    }

    pub(crate) async fn refresh_pr(
        self: &Arc<Self>,
        id: &SessionId,
    ) -> Result<Reply, RequestError> {
        let _busy = Busy::new(self);
        let (record, repo) = self.snapshot(id).ok_or(RequestError::UnknownSession)?;
        let Some(number) = record.pr_number() else {
            return Err(refused("the Session has no PR"));
        };
        let json = gh(
            &repo,
            &["pr", "view", &number.to_string(), "--json", PR_FIELDS],
            POLL_TIMEOUT,
        )
        .await
        .map_err(refused)?;
        let pr: GhPr = serde_json::from_str(&json)
            .map_err(|err| refused(format!("unexpected gh output: {err}")))?;
        if pr.state == GhState::Merged {
            let (_claim, _, _) = self.claim(id, |live| match live.waits_on_pr() {
                true => Ok(()),
                false => Err("the Session is no longer waiting on its PR".into()),
            })?;
            self.end_session(id, PhaseEvent::PrMerged).await?;
            return Ok(Reply::Done);
        }
        let status = PrStatus {
            number,
            checks: pr.checks(),
            review: pr.review(),
            new_comments: 0,
            state: match pr.state {
                GhState::Closed => PrState::Closed,
                _ => PrState::Open,
            },
        };
        let comments = pr.comment_count();
        self.update(id, |live| {
            if !live.exclusive && live.waits_on_pr() && live.record.pr_number() == Some(number) {
                live.update_pr(status, comments);
            }
            Ok(())
        })?;
        Ok(Reply::Done)
    }

    pub(crate) async fn poll_prs(self: Arc<Self>) {
        let limit = Arc::new(Semaphore::new(POLLS_AT_ONCE));
        loop {
            let waiting: Vec<SessionId> = self
                .lock()
                .sessions
                .values()
                .filter(|live| live.waits_on_pr())
                .map(|live| live.record.id.clone())
                .collect();
            let mut polls = JoinSet::new();
            for id in waiting {
                let daemon = self.clone();
                let limit = limit.clone();
                polls.spawn(async move {
                    let _permit = limit.acquire_owned().await;
                    if let Err(err) = daemon.refresh_pr(&id).await {
                        eprintln!("orch daemon: polling the PR of {}: {err}", id.as_str());
                    }
                });
            }
            while polls.join_next().await.is_some() {}
            tokio::time::sleep(self.config.pr_poll_interval).await;
        }
    }
}
