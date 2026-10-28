use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use orch_config::ConfigLoader;
use orch_core::{ChecksState, PhaseEvent, PrState, PrStatus, ReviewDecision, SessionId};
use orch_holder::GhOutput;
use orch_protocol::{Reply, RequestError};
use orch_store::SessionRecord;
use serde::Deserialize;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::broker::origin_url;
use crate::broker_gh::GitHubRepo;
use crate::lifecycle::{Busy, refused, with_git};
use crate::state::{CommentCursor, Daemon, Live, session_worktree};
use crate::subprocess;

const GH_TIMEOUT: Duration = Duration::from_secs(120);
const POLL_TIMEOUT: Duration = Duration::from_secs(30);
const POLLS_AT_ONCE: usize = 4;
const PR_FIELDS: &str = "state,statusCheckRollup,reviewDecision,comments,reviews,baseRefName";
const LOOKUP_FIELDS: &str = "number,title,state,baseRefName,headRefName,isCrossRepository";
const OPEN_PRS_SHOWN: &str = "10";

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
    #[serde(default)]
    base_ref_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhFound {
    number: u64,
    #[serde(default)]
    title: String,
    state: GhState,
    base_ref_name: String,
    head_ref_name: String,
    #[serde(default)]
    is_cross_repository: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BranchPr {
    pub(crate) number: u64,
    pub(crate) title: String,
    pub(crate) state: PrState,
    pub(crate) base: String,
    pub(crate) head: String,
}

impl BranchPr {
    pub(crate) fn is_open(&self) -> bool {
        self.state == PrState::Open
    }
}

impl TryFrom<GhFound> for BranchPr {
    type Error = String;

    fn try_from(found: GhFound) -> Result<Self, String> {
        let number = found.number;
        let state = match found.state {
            GhState::Open => PrState::Open,
            GhState::Closed => PrState::Closed,
            GhState::Merged => return Err(format!("PR #{number} is already merged")),
            GhState::Unknown => return Err(format!("PR #{number} is in an unknown state")),
        };
        if found.is_cross_repository {
            return Err(format!("PR #{number}'s Branch is in a fork"));
        }
        Ok(BranchPr {
            number,
            title: found.title,
            state,
            base: found.base_ref_name,
            head: found.head_ref_name,
        })
    }
}

pub(crate) enum PrQueryError {
    NoGitHub,
    Failed(String),
}

impl fmt::Display for PrQueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrQueryError::NoGitHub => write!(f, "origin is not on GitHub or gh is not installed"),
            PrQueryError::Failed(err) => write!(f, "{err}"),
        }
    }
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
    OriginAhead(String),
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
            PushError::OriginAhead(branch) => {
                write!(f, "origin has commits {branch} lacks: pull or rebase first")
            }
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
    if git.origin_is_ahead(branch)? {
        return Err(PushError::OriginAhead(branch.into()));
    }
    git.push(branch)?;
    Ok(())
}

pub(crate) fn pr_number(output: &str) -> Option<u64> {
    let url = output.lines().rev().find(|line| !line.trim().is_empty())?;
    url.trim()
        .trim_end_matches('/')
        .rsplit('/')
        .next()?
        .parse()
        .ok()
}

async fn gh(repo: &Path, args: &[&str], limit: Duration) -> Result<String, String> {
    let mut command = crate::subprocess::command("gh");
    command.args(args).current_dir(repo);
    let what = format!(
        "gh {}",
        args.iter().take(2).copied().collect::<Vec<_>>().join(" ")
    );
    subprocess::run(command, None, limit, &what).await
}

pub(crate) async fn gh_brokered(repo: &Path, args: &[String]) -> Result<GhOutput, String> {
    let mut command = crate::subprocess::command("gh");
    command
        .args(args)
        .current_dir(repo)
        .env("GH_PROMPT_DISABLED", "1");
    let output = subprocess::output(command, None, GH_TIMEOUT, "gh").await?;
    Ok(GhOutput {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        code: output.status.code().unwrap_or(-1),
    })
}

pub(crate) async fn pr_by_number(repo: &Path, number: u64) -> Result<BranchPr, String> {
    let json = gh(
        repo,
        &["pr", "view", &number.to_string(), "--json", LOOKUP_FIELDS],
        POLL_TIMEOUT,
    )
    .await
    .map_err(|err| format!("PR #{number} wasn't found: {err}"))?;
    let found: GhFound =
        serde_json::from_str(&json).map_err(|err| format!("unexpected gh output: {err}"))?;
    BranchPr::try_from(found)
}

pub(crate) async fn open_prs(repo: &Path, branch: &str) -> Result<Vec<BranchPr>, PrQueryError> {
    let on_github = origin_url(repo)
        .await
        .as_deref()
        .and_then(GitHubRepo::from_remote)
        .is_some();
    if !on_github || !subprocess::on_path("gh") {
        return Err(PrQueryError::NoGitHub);
    }
    let listed = gh(
        repo,
        &[
            "pr",
            "list",
            "--head",
            branch,
            "--state",
            "open",
            "--json",
            LOOKUP_FIELDS,
            "--limit",
            OPEN_PRS_SHOWN,
        ],
        POLL_TIMEOUT,
    )
    .await
    .map_err(PrQueryError::Failed)?;
    let found: Vec<GhFound> = serde_json::from_str(&listed)
        .map_err(|err| PrQueryError::Failed(format!("unexpected gh output: {err}")))?;
    Ok(found
        .into_iter()
        .filter_map(|found| BranchPr::try_from(found).ok())
        .filter(BranchPr::is_open)
        .collect())
}

fn refused_after_commit(
    committed: Option<&str>,
    title: &str,
    err: impl fmt::Display,
) -> RequestError {
    match committed {
        Some(commit) => refused(format!(
            "the uncommitted changes were committed as {} \"{title}\" and stay on the Branch, but {err}",
            &commit[..commit.len().min(12)]
        )),
        None => refused(err),
    }
}

async fn prepare_followed_base(repo: &Path, base: &str) -> Option<String> {
    let wanted = base.to_string();
    let prepared = with_git(repo.to_path_buf(), move |git| {
        git.fast_forward_from_origin(&wanted)?;
        Ok::<_, orch_git::Error>(git.branch_exists(&wanted))
    })
    .await;
    match prepared {
        Ok(Ok(true)) => None,
        Ok(Ok(false)) => Some(format!("there is no Branch {base} locally or on origin")),
        Ok(Err(err)) => Some(err.to_string()),
        Err(err) => Some(err.to_string()),
    }
}

pub(crate) enum PrToPush {
    Recorded(u64),
    Found(BranchPr),
}

impl PrToPush {
    pub(crate) fn number(&self) -> u64 {
        match self {
            PrToPush::Recorded(number) => *number,
            PrToPush::Found(pr) => pr.number,
        }
    }
}

fn known_pr_base(record: &SessionRecord) -> String {
    record
        .pr_base
        .clone()
        .unwrap_or_else(|| record.base.clone())
}

enum BaseSync {
    Keep,
    Follow(String),
    Push(String),
}

fn base_sync(record: &SessionRecord, known: &str, on_github: Option<&str>) -> BaseSync {
    match on_github {
        Some(base) if base != known => BaseSync::Follow(base.into()),
        Some(base) if base != record.base => BaseSync::Push(record.base.clone()),
        _ => BaseSync::Keep,
    }
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
    async fn commit_and_push(
        &self,
        record: &SessionRecord,
        repo: &Path,
        title: &str,
    ) -> Result<Option<String>, RequestError> {
        let worktree = session_worktree(record);
        let message = title.to_string();
        let committed = with_git(repo.to_path_buf(), move |git| {
            git.commit_worktree(&worktree, &message)
        })
        .await?
        .map_err(refused)?;
        let loader = self.config.loader.clone();
        let (branch, base) = (record.branch.clone(), record.base.clone());
        with_git(repo.to_path_buf(), move |git| {
            push_for_pr(git, &loader, &branch, &base)
        })
        .await?
        .map_err(|err| refused_after_commit(committed.as_deref(), title, err))?;
        Ok(committed)
    }

    pub(crate) async fn open_pr(
        self: &Arc<Self>,
        id: &SessionId,
        record: SessionRecord,
        repo: PathBuf,
        title: String,
        body: String,
    ) -> Result<Reply, RequestError> {
        let committed = self.commit_and_push(&record, &repo, &title).await?;
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
        .await;
        let failed = |err: String| refused_after_commit(committed.as_deref(), &title, err);
        let committed_changes = committed.is_some();
        match created {
            Ok(created) => {
                let number = pr_number(&created)
                    .ok_or_else(|| failed(format!("gh printed no PR link: {}", created.trim())))?;
                self.record_pr(id, number)?;
                Ok(Reply::PrOpened {
                    number,
                    committed_changes,
                })
            }
            Err(err) => {
                let adopted = open_prs(&repo, &record.branch)
                    .await
                    .ok()
                    .and_then(|prs| prs.into_iter().next())
                    .ok_or_else(|| failed(err))?;
                self.adopt_pr(id, &repo, &adopted).await?;
                Ok(Reply::Pushed {
                    number: adopted.number,
                    committed_changes,
                })
            }
        }
    }

    pub(crate) async fn push_to_pr(
        self: &Arc<Self>,
        id: &SessionId,
        record: SessionRecord,
        repo: PathBuf,
        title: String,
        pr: PrToPush,
    ) -> Result<Reply, RequestError> {
        let known = known_pr_base(&record);
        let committed = self.commit_and_push(&record, &repo, &title).await?;
        let number = match pr {
            PrToPush::Recorded(number) => {
                let on_github = pr_by_number(&repo, number).await.ok().map(|pr| pr.base);
                self.sync_pr_base(id, &repo, number, &known, on_github.as_deref(), true)
                    .await;
                number
            }
            PrToPush::Found(found) => {
                self.adopt_pr(id, &repo, &found).await?;
                found.number
            }
        };
        Ok(Reply::Pushed {
            number,
            committed_changes: committed.is_some(),
        })
    }

    async fn adopt_pr(
        &self,
        id: &SessionId,
        repo: &Path,
        pr: &BranchPr,
    ) -> Result<(), RequestError> {
        let moved = self
            .snapshot(id)
            .is_some_and(|(record, _)| record.base != pr.base);
        let problem = match moved {
            true => prepare_followed_base(repo, &pr.base).await,
            false => None,
        };
        self.update(id, |live| {
            if moved {
                live.follow_base(&pr.base, problem);
            }
            live.record.pr_base = Some(pr.base.clone());
            live.comments = CommentCursor::default();
            live.transition(PhaseEvent::PrOpened { number: pr.number })
        })
    }

    async fn sync_pr_base(
        &self,
        id: &SessionId,
        repo: &Path,
        number: u64,
        known: &str,
        on_github: Option<&str>,
        claimed: bool,
    ) {
        let may_sync = |live: &Live| {
            live.record.pr_number() == Some(number)
                && known_pr_base(&live.record) == known
                && (claimed || (!live.exclusive && live.waits_on_pr()))
        };
        let sync = {
            let state = self.lock();
            match state.sessions.get(id) {
                Some(live) if may_sync(live) => base_sync(&live.record, known, on_github),
                _ => BaseSync::Keep,
            }
        };
        match sync {
            BaseSync::Keep => {}
            BaseSync::Follow(base) => {
                let problem = prepare_followed_base(repo, &base).await;
                let _ = self.update(id, |live| {
                    if may_sync(live) {
                        live.follow_base(&base, problem);
                        live.record.pr_base = Some(base.clone());
                    }
                    Ok(())
                });
            }
            BaseSync::Push(base) => {
                let pushed = retarget_pr(repo, number, &base).await;
                let _ = self.update(id, |live| {
                    match &pushed {
                        Ok(()) if live.record.base == base => {
                            live.record.pr_base = Some(base.clone());
                        }
                        Ok(()) => {}
                        Err(err) => {
                            live.last_error = Some(format!(
                                "retargeting PR #{number} onto {base} failed: {err}"
                            ));
                        }
                    }
                    Ok(())
                });
            }
        }
    }

    pub(crate) fn pr_retargeted(&self, id: &SessionId, base: &str) {
        let _ = self.update(id, |live| {
            if live.record.base == base {
                live.record.pr_base = Some(base.into());
            }
            Ok(())
        });
    }

    pub(crate) fn record_pr(&self, id: &SessionId, number: u64) -> Result<(), RequestError> {
        self.update(id, |live| {
            live.comments = CommentCursor::opened();
            live.record.pr_base = Some(live.record.base.clone());
            live.transition(PhaseEvent::PrOpened { number })
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
        let known = known_pr_base(&record);
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
        self.sync_pr_base(
            id,
            &repo,
            number,
            &known,
            pr.base_ref_name.as_deref(),
            false,
        )
        .await;
        Ok(Reply::Done)
    }

    pub(crate) async fn poll_prs(self: Arc<Self>) {
        loop {
            self.poll_waiting_prs().await;
            tokio::time::sleep(self.config.pr_poll_interval).await;
        }
    }

    pub(crate) async fn poll_waiting_prs(self: &Arc<Self>) {
        let limit = Arc::new(Semaphore::new(POLLS_AT_ONCE));
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
    }
}
