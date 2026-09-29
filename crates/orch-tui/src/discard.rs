use crossterm::event::{KeyCode, KeyEvent};
use orch_core::SessionId;
use orch_protocol::CommitView;

const SHORT_ID: usize = 7;

pub(crate) struct DiscardConfirm {
    pub session: SessionId,
    pub uncommitted: Vec<String>,
    pub unlanded: Vec<String>,
}

impl DiscardConfirm {
    pub fn new(session: SessionId, uncommitted: Vec<String>, unlanded: Vec<CommitView>) -> Self {
        let unlanded = unlanded
            .into_iter()
            .map(|commit| {
                let short = &commit.id[..commit.id.len().min(SHORT_ID)];
                format!("{short} {}", commit.subject)
            })
            .collect();
        Self {
            session,
            uncommitted,
            unlanded,
        }
    }

    pub fn confirms(key: KeyEvent) -> bool {
        key.code == KeyCode::Char('y')
    }
}
