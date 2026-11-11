use std::sync::Arc;

use orch_core::{PhaseEvent, SessionId};
use orch_git::DiscardPreview;
use orch_protocol::{CommitView, Reply, RequestError};

use crate::children::open_descendants;
use crate::lifecycle::{Busy, refused, with_git};
use crate::state::{Daemon, gate_message, session_worktree};

impl Daemon {
    pub(crate) async fn discard_preview(&self, id: &SessionId) -> Result<Reply, RequestError> {
        let (record, repo) = self.snapshot(id).ok_or(RequestError::UnknownSession)?;
        let worktree = session_worktree(&record);
        let preview = with_git(repo, move |git| git.discard_preview(&worktree))
            .await?
            .map_err(refused)?;
        Ok(preview_reply(preview, self.open_children_names(id)))
    }

    /// Discards the Session and, first, its open descendants, deepest first.
    pub(crate) async fn discard(
        self: &Arc<Self>,
        id: &SessionId,
        skip_teardown: bool,
    ) -> Result<Reply, RequestError> {
        let _busy = Busy::new(self);
        let descendants = open_descendants(&self.lock().sessions, id);
        for child in &descendants {
            self.discard_one(child, skip_teardown)
                .await
                .map_err(|err| refused(format!("Discarding a Child Session failed: {err}")))?;
        }
        self.discard_one(id, skip_teardown).await
    }

    async fn discard_one(
        self: &Arc<Self>,
        id: &SessionId,
        skip_teardown: bool,
    ) -> Result<Reply, RequestError> {
        let (_claim, _, _) = self.claim(id, |live| {
            live.status
                .check_discard()
                .map(drop)
                .map_err(|refusal| gate_message("Discarding", refusal))
        })?;
        if !skip_teardown {
            let (_, repo) = self.snapshot(id).ok_or(RequestError::UnknownSession)?;
            self.check_teardown(&repo).await?;
        }
        self.end_session(id, PhaseEvent::Discarded).await?;
        Ok(Reply::Done)
    }
}

pub(crate) fn preview_reply(preview: DiscardPreview, children: Vec<String>) -> Reply {
    Reply::DiscardPreview {
        uncommitted: preview.uncommitted,
        unlanded: preview
            .unlanded
            .into_iter()
            .map(|commit| CommitView {
                id: commit.id,
                subject: commit.subject,
            })
            .collect(),
        branch_kept: preview.branch_kept,
        children,
    }
}
