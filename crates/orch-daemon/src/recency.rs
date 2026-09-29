use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default, Clone)]
pub(crate) struct UseClock(Arc<AtomicU64>);

impl UseClock {
    pub(crate) fn stamp(&self) -> LastUsed {
        let last_used = LastUsed {
            clock: self.0.clone(),
            at: Arc::default(),
        };
        last_used.touch();
        last_used
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LastUsed {
    clock: Arc<AtomicU64>,
    at: Arc<AtomicU64>,
}

impl LastUsed {
    pub(crate) fn touch(&self) {
        let now = self.clock.fetch_add(1, Ordering::Relaxed) + 1;
        self.at.fetch_max(now, Ordering::Relaxed);
    }

    pub(crate) fn at(&self) -> u64 {
        self.at.load(Ordering::Relaxed)
    }
}
