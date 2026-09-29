use std::collections::HashMap;

use orch_agent::GuardAnswer;
use tokio::sync::oneshot;

use crate::GuardId;

pub(crate) type GuardReply = oneshot::Sender<GuardAnswer>;

#[derive(Default)]
pub(crate) struct GuardTable {
    pending: HashMap<GuardId, (GuardReply, bool)>,
    next: GuardId,
}

impl GuardTable {
    pub(crate) fn open(&mut self, reply: GuardReply) -> GuardId {
        self.next += 1;
        self.pending.insert(self.next, (reply, false));
        self.next
    }

    pub(crate) fn hold(&mut self, id: GuardId) {
        if let Some((_, held)) = self.pending.get_mut(&id) {
            *held = true;
        }
    }

    pub(crate) fn is_unheld(&self, id: GuardId) -> bool {
        self.pending.get(&id).is_some_and(|(_, held)| !held)
    }

    pub(crate) fn take(&mut self, id: GuardId) -> Option<GuardReply> {
        self.pending.remove(&id).map(|(reply, _)| reply)
    }

    pub(crate) fn ids(&self) -> Vec<GuardId> {
        self.pending.keys().copied().collect()
    }
}
