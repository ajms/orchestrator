use std::path::PathBuf;
use std::time::Duration;

use orch_agent::{Argv, Draft, DraftInput};
use orch_core::SessionId;
use orch_holder::SESSION_ENV;
use orch_protocol::{LandingMode, Reply, RequestError};
use orch_store::SessionRecord;

use crate::agents::installed_adapter;
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

async fn append_base_diff(
    prompt: String,
    repo: PathBuf,
    record: &SessionRecord,
) -> Result<String, RequestError> {
    let worktree = session_worktree(record);
    let diff = with_git(repo, move |git| {
        let snapshot = git.review_snapshot(&worktree)?;
        git.diff(&snapshot.merge_base, &snapshot.tree)
    })
    .await?
    .map_err(refused)?;
    Ok(format!(
        "{prompt}\n\nThe changes are this diff against {}, so you need no tools:\n\n{diff}",
        record.base
    ))
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
        let config = self.repo_config(&repo).await?;
        let agent = config
            .agent(&record.agent)
            .map_err(|_| untrusted(&repo, &config))?;
        let adapter = installed_adapter(&agent, &repo).map_err(refused)?;
        let Some(Draft {
            argv: Argv { program, args },
            input,
        }) = adapter.draft(record.latest_conversation())
        else {
            return Ok(self.default_draft(id, &record.slug).await);
        };
        let mut prompt = instruction(mode, &record.base);
        if input == DraftInput::InstructionAndBaseDiff {
            prompt = append_base_diff(prompt, repo, &record).await?;
        }
        let mut command = crate::subprocess::command(program);
        command
            .args(&agent.args)
            .args(args)
            .current_dir(&record.worktree)
            .env_remove(SESSION_ENV);
        let drafted = subprocess::run(command, Some(prompt), DRAFT_TIMEOUT, "the Agent's draft")
            .await
            .map_err(refused)?;
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
