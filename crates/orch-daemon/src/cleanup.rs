use std::path::Path;

use orch_core::SessionId;
use orch_git::{CleanupCheckpoint, Script, ScriptOutcome, SessionWorktree};
use serde::{Deserialize, Serialize};

use crate::state::Daemon;

const MARKER: &str = "cleanup.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct CleanupMarker {
    worktree: Option<u64>,
    branch_tip: Option<String>,
    pub(crate) teardown_done: bool,
}

impl CleanupMarker {
    pub(crate) fn checkpoint(&self) -> CleanupCheckpoint {
        CleanupCheckpoint {
            worktree: self.worktree,
            branch_tip: self.branch_tip.clone(),
        }
    }

    fn record(&mut self, git: &orch_git::Repo, worktree: &SessionWorktree, dir: &Path) {
        let checkpoint = git.cleanup_checkpoint(worktree);
        self.worktree = checkpoint.worktree;
        self.branch_tip = checkpoint.branch_tip;
        let written = std::fs::create_dir_all(dir).and_then(|()| {
            let json = serde_json::to_vec(self).map_err(std::io::Error::other)?;
            std::fs::write(dir.join(MARKER), json)
        });
        if let Err(err) = written {
            eprintln!(
                "orch daemon: recording the cleanup in {}: {err}",
                dir.display()
            );
        }
    }

    pub(crate) fn read(dir: &Path) -> Option<CleanupMarker> {
        let json = std::fs::read(dir.join(MARKER)).ok()?;
        serde_json::from_slice(&json).ok()
    }
}

pub(crate) fn remove_session_dir(dir: &Path) -> Result<(), String> {
    match std::fs::remove_dir_all(dir) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("removing {}: {err}", dir.display()))
        }
        _ => Ok(()),
    }
}

pub(crate) fn clean_up_ended(
    git: &orch_git::Repo,
    worktree: &SessionWorktree,
    teardown: Option<&Script>,
    dir: &Path,
) -> Result<Option<ScriptOutcome>, orch_git::Error> {
    let mut marker = CleanupMarker::default();
    marker.record(git, worktree, dir);
    let outcome = teardown
        .filter(|_| worktree.path.is_dir())
        .map(|script| script.run(&worktree.path));
    marker.teardown_done = true;
    marker.record(git, worktree, dir);
    match git.remove_session_worktree(worktree, None) {
        Ok(_) => {
            let _ = remove_session_dir(dir);
            Ok(outcome)
        }
        Err(err) => {
            marker.record(git, worktree, dir);
            Err(err)
        }
    }
}

impl Daemon {
    pub(crate) async fn forget_session(&self, id: &SessionId) {
        self.store.flush().await;
        self.lock().remove_session(id);
    }

    pub(crate) async fn free_port_block(&self, id: &SessionId) -> Result<(), String> {
        let session = id.clone();
        self.store
            .call(move |store| store.free_port_block(&session))
            .await
            .map_err(|err| err.to_string())?
            .map_err(|err| format!("freeing the Port block failed: {err}"))
    }
}
