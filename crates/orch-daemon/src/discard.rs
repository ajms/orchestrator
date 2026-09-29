use std::sync::Arc;

use orch_core::{PhaseEvent, SessionId};
use orch_protocol::{CommitView, Reply, RequestError};

use crate::lifecycle::{Busy, refused, with_git};
use crate::state::{Daemon, gate_message, session_worktree};

impl Daemon {
    pub(crate) async fn discard_preview(&self, id: &SessionId) -> Result<Reply, RequestError> {
        let (record, repo) = self.snapshot(id).ok_or(RequestError::UnknownSession)?;
        let worktree = session_worktree(&record);
        let preview = with_git(repo, move |git| git.discard_preview(&worktree))
            .await?
            .map_err(refused)?;
        Ok(Reply::DiscardPreview {
            uncommitted: preview.uncommitted,
            unlanded: preview
                .unlanded
                .into_iter()
                .map(|commit| CommitView {
                    id: commit.id,
                    subject: commit.subject,
                })
                .collect(),
        })
    }

    pub(crate) async fn discard(
        self: &Arc<Self>,
        id: &SessionId,
        skip_teardown: bool,
    ) -> Result<Reply, RequestError> {
        let _busy = Busy::new(self);
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
