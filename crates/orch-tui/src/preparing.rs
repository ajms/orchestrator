use std::time::Instant;

use orch_core::SessionId;
use orch_git::slugify;
use orch_protocol::CreateSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PreparingId(pub u64);

pub(crate) struct Preparing {
    pub id: PreparingId,
    pub create: CreateSession,
    pub slug: String,
    pub since: Instant,
    pub session: Option<SessionId>,
    pub error: Option<String>,
    pub needs_trust: bool,
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
            error: None,
            needs_trust: false,
        }
    }
}
