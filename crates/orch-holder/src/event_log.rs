use std::collections::VecDeque;

use crate::{GuardId, HolderEvent};

pub(crate) struct EventLog {
    entries: VecDeque<(u64, HolderEvent)>,
    next_seq: u64,
    capacity: usize,
}

impl EventLog {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            next_seq: 1,
            capacity: capacity.max(1),
        }
    }

    pub(crate) fn push(&mut self, event: HolderEvent) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        if matches!(event, HolderEvent::Tap { .. }) {
            self.entries
                .retain(|(_, logged)| !matches!(logged, HolderEvent::Tap { .. }));
        }
        self.entries.push_back((seq, event));
        while self.entries.len() > self.capacity {
            self.entries.pop_front();
        }
        seq
    }

    pub(crate) fn ack(&mut self, through: u64) {
        while self.entries.front().is_some_and(|(seq, _)| *seq <= through) {
            self.entries.pop_front();
        }
    }

    pub(crate) fn unacked(&self) -> impl Iterator<Item = &(u64, HolderEvent)> {
        self.entries.iter()
    }

    pub(crate) fn settle_guard(&mut self, id: GuardId) {
        for (_, event) in &mut self.entries {
            if let HolderEvent::Hook { guard, .. } = event
                && *guard == Some(id)
            {
                *guard = None;
            }
        }
    }
}
