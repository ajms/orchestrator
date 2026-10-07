use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

use orch_agent::{Adapter, Argv, Draft, DraftInput, DraftOutcome};
use orch_config::AgentConfig;
use orch_core::SessionId;
use orch_holder::SESSION_ENV;
use orch_protocol::{LandingMode, Reply, RequestError};
use orch_store::SessionRecord;

use crate::agents::installed_adapter;
use crate::lifecycle::{PROMPT_FILE, refused, untrusted, with_git};
use crate::state::{Daemon, session_worktree};
use crate::subprocess;

const DRAFT_TIMEOUT: Duration = Duration::from_secs(180);
const DIFF_LIMIT: usize = 100 * 1024;
const DRAFT_LABEL: &str = "the Agent's draft";

fn instruction(mode: LandingMode, base: &str) -> String {
    match mode {
        LandingMode::Squash => format!(
            "Write a git commit message for all changes in this Worktree compared with the Base branch {base}, including uncommitted and untracked files. Reply with the commit message only: a summary line of at most 72 characters, a blank line, then a short body."
        ),
        LandingMode::Pr => format!(
            "Write a pull request title and description for all changes on this Branch compared with the Base branch {base}. Reply with the title only on the first line, a blank line, then the description in Markdown."
        ),
    }
}

fn with_diff(instruction: String, base: &str, diff: &str) -> String {
    format!(
        "{instruction}\n\nThe changes are this diff against {base}, so you need no tools:\n\n{diff}"
    )
}

fn truncated(patch: &str) -> String {
    if patch.len() <= DIFF_LIMIT {
        return patch.into();
    }
    let cut = patch.as_bytes()[..DIFF_LIMIT]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .unwrap_or(0);
    format!(
        "{}\n[diff truncated: {} more bytes left out; the stat above lists every changed file]",
        &patch[..cut],
        patch.len() - cut
    )
}

async fn base_diff(repo: PathBuf, record: &SessionRecord) -> Result<String, RequestError> {
    let worktree = session_worktree(record);
    let (stat, patch) = with_git(repo, move |git| {
        let snapshot = git.review_snapshot(&worktree)?;
        let stat = git.diff_stat(&snapshot.merge_base, &snapshot.tree)?;
        let patch = git.diff(&snapshot.merge_base, &snapshot.tree)?;
        Ok::<_, orch_git::Error>((stat, patch))
    })
    .await?
    .map_err(refused)?;
    Ok(format!("{stat}\n\n{}", truncated(&patch)))
}

fn drafted(adapter: &Adapter, output: &Output) -> Result<String, String> {
    match (
        adapter.decode_draft(&String::from_utf8_lossy(&output.stdout)),
        output.status.success(),
    ) {
        (DraftOutcome::Failed(error), _) => Err(format!("{DRAFT_LABEL} failed: {error}")),
        (DraftOutcome::Drafted(text), true) => Ok(text),
        (DraftOutcome::NoResult, true) => Err(format!("{DRAFT_LABEL} ended without a result")),
        (_, false) => Err(subprocess::failed(DRAFT_LABEL, output)),
    }
}

fn split_draft(text: &str) -> Reply {
    let text = text.trim();
    let (title, body) = text.split_once('\n').unwrap_or((text, ""));
    Reply::Drafted {
        title: title.trim().into(),
        body: body.trim().into(),
    }
}

impl Daemon {
    pub(crate) async fn draft(
        &self,
        id: &SessionId,
        mode: LandingMode,
    ) -> Result<Reply, RequestError> {
        let (record, repo) = self.snapshot(id).ok_or(RequestError::UnknownSession)?;
        let (agent, adapter) = match self.draft_agent(&record, &repo).await {
            Ok(found) => found,
            Err(_) if record.latest_conversation().is_none() => {
                return Ok(self.default_draft(id, &record.slug).await);
            }
            Err(err) => return Err(err),
        };
        let Some(Draft {
            argv: Argv { program, args },
            input,
        }) = adapter.draft(record.latest_conversation())
        else {
            return Ok(self.default_draft(id, &record.slug).await);
        };
        let instruction = instruction(mode, &record.base);
        let prompt = match input {
            DraftInput::Instruction => instruction,
            DraftInput::InstructionAndBaseDiff => {
                let diff = base_diff(repo, &record).await?;
                with_diff(instruction, &record.base, &diff)
            }
        };
        let mut command = subprocess::command(program);
        command
            .args(&agent.args)
            .args(args)
            .current_dir(&record.worktree)
            .env_remove(SESSION_ENV);
        let output = subprocess::output(
            command,
            Some(adapter.encode_draft(&prompt)),
            DRAFT_TIMEOUT,
            DRAFT_LABEL,
        )
        .await
        .map_err(refused)?;
        Ok(split_draft(&drafted(&adapter, &output).map_err(refused)?))
    }

    async fn draft_agent(
        &self,
        record: &SessionRecord,
        repo: &Path,
    ) -> Result<(AgentConfig, Adapter), RequestError> {
        let config = self.repo_config(repo).await?;
        let agent = config
            .agent(&record.agent)
            .map_err(|_| untrusted(repo, &config))?;
        let adapter = installed_adapter(&agent, repo).map_err(refused)?;
        Ok((agent, adapter))
    }

    async fn default_draft(&self, id: &SessionId, slug: &str) -> Reply {
        let prompt_file = self.session_dir(id).join(PROMPT_FILE);
        let prompt = tokio::task::spawn_blocking(move || std::fs::read_to_string(prompt_file))
            .await
            .ok()
            .and_then(Result::ok)
            .filter(|prompt| !prompt.trim().is_empty());
        split_draft(&prompt.unwrap_or_else(|| slug.replace('-', " ")))
    }
}
