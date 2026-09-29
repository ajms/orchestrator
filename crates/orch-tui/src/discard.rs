use crossterm::event::{KeyCode, KeyEvent};
use orch_core::SessionId;
use std::path::PathBuf;

use orch_protocol::{CommitView, LeftoverView};

const SHORT_ID: usize = 7;

pub(crate) enum DiscardTarget {
    Session(SessionId),
    Leftover {
        repo: PathBuf,
        leftover: LeftoverView,
    },
}

pub(crate) struct DiscardConfirm {
    pub target: DiscardTarget,
    pub question: String,
    pub uncommitted: Vec<String>,
    pub unlanded: Vec<String>,
}

impl DiscardConfirm {
    pub fn new(
        target: DiscardTarget,
        question: String,
        uncommitted: Vec<String>,
        unlanded: Vec<CommitView>,
    ) -> Self {
        let unlanded = unlanded
            .into_iter()
            .map(|commit| {
                let short = &commit.id[..commit.id.len().min(SHORT_ID)];
                format!("{short} {}", commit.subject)
            })
            .collect();
        Self {
            target,
            question,
            uncommitted,
            unlanded,
        }
    }

    pub fn confirms(key: KeyEvent) -> bool {
        key.code == KeyCode::Char('y')
    }
}
