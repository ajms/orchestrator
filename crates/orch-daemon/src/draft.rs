use std::path::PathBuf;
use std::time::Duration;

use orch_agent::{Argv, DraftRun, DraftSpec};
use orch_config::xdg;
use orch_core::SessionId;
use orch_protocol::{LandingMode, Reply, RequestError};
use orch_store::SessionRecord;

use crate::agents::adapter_for;
use crate::lifecycle::{PROMPT_FILE, refused, untrusted, with_git};
use crate::state::{Daemon, session_worktree};
use crate::subprocess;

const DRAFT_TIMEOUT: Duration = Duration::from_secs(180);

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

async fn base_diff(record: &SessionRecord, repo: PathBuf) -> Result<String, RequestError> {
    let worktree = session_worktree(record);
    with_git(repo, move |git| {
        let status = git.status(&worktree)?;
        let snapshot = git.review_snapshot(&worktree)?;
        let diff = git.diff(&snapshot.merge_base, &snapshot.tree)?;
        Ok::<_, orch_git::Error>(format!(
            "\n\n`git status --short`:\n{status}\n\nDiff against the Base branch {}:\n{diff}\n",
            worktree.base
        ))
    })
    .await?
    .map_err(refused)
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
        let Some(conversation) = record.latest_conversation().cloned() else {
            return Ok(self.default_draft(id, &record.slug).await);
        };
        let config = self.repo_config(&repo).await?;
        let agent = config
            .agent(&record.agent)
            .map_err(|_| untrusted(&repo, &config))?;
        let adapter = adapter_for(&agent).map_err(refused)?;
        let scratch = tempfile::Builder::new()
            .prefix("orch-draft-")
            .tempdir()
            .map_err(refused)?;
        let spec = DraftSpec {
            conversation,
            worktree: record.worktree.clone(),
            scratch: scratch.path().into(),
        };
        let planned = tokio::task::spawn_blocking(move || adapter.draft(&spec, &xdg::process_env))
            .await
            .map_err(refused)?
            .map_err(refused)?;
        let Some(DraftRun {
            argv: Argv { program, args },
            env,
            diff,
        }) = planned
        else {
            return Ok(self.default_draft(id, &record.slug).await);
        };
        let mut input = instruction(mode, &record.base);
        if diff {
            input.push_str(&base_diff(&record, repo).await?);
        }
        let mut command = crate::subprocess::command(program);
        command
            .args(&agent.args)
            .args(args)
            .envs(env)
            .current_dir(&record.worktree);
        let drafted = subprocess::run(
            command,
            Some(input.into()),
            DRAFT_TIMEOUT,
            "the Agent's draft",
        )
        .await
        .map_err(refused)?;
        drop(scratch);
        Ok(split_draft(&drafted))
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
