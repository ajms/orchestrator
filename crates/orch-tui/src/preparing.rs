use std::time::Instant;

use orch_core::SessionId;
use orch_git::slugify;
use orch_protocol::CreateSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PreparingId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PreparingState {
    Waiting,
    NeedsTrust,
    Failed(String),
}

pub(crate) struct Preparing {
    pub id: PreparingId,
    pub create: CreateSession,
    pub slug: String,
    pub since: Instant,
    pub session: Option<SessionId>,
    pub state: PreparingState,
}

impl Preparing {
    pub fn new(id: PreparingId, create: CreateSession, prefix: &str, since: Instant) -> Self {
        let slug = match &create.branch {
            Some(branch) => slugify(branch.strip_prefix(prefix).unwrap_or(branch)),
            None => slugify(&create.prompt),
        };
        Self {
            id,
            create,
            slug,
            since,
            session: None,
            state: PreparingState::Waiting,
        }
    }

    pub fn is_counting(&self) -> bool {
        self.state == PreparingState::Waiting
    }

    pub fn failure(&self) -> Option<&str> {
        match &self.state {
            PreparingState::Failed(reason) => Some(reason),
            _ => None,
        }
    }

    pub fn fail(&mut self, reason: impl Into<String>) {
        self.state = PreparingState::Failed(reason.into());
    }
}
