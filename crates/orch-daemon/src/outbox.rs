use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use orch_core::SessionId;
use orch_protocol::{FromDaemon, SessionView};
use tokio::sync::Notify;

const MAX_QUEUED: usize = 1024;

#[derive(Default)]
pub(crate) struct Outbox {
    queue: Mutex<Queue>,
    ready: Notify,
}

#[derive(Default)]
struct Queue {
    messages: VecDeque<FromDaemon>,
    changed: VecDeque<SessionId>,
    views: HashMap<SessionId, Box<SessionView>>,
    overflowed: bool,
}

impl Outbox {
    fn queue(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn send(&self, message: FromDaemon) {
        let mut queue = self.queue();
        if queue.messages.len() >= MAX_QUEUED {
            queue.overflowed = true;
        } else {
            queue.messages.push_back(message);
        }
        drop(queue);
        self.ready.notify_one();
    }

    pub(crate) fn session_changed(&self, view: Box<SessionView>) {
        let mut queue = self.queue();
        let id = view.id.clone();
        if queue.views.insert(id.clone(), view).is_none() {
            queue.changed.push_back(id);
        }
        drop(queue);
        self.ready.notify_one();
    }

    pub(crate) async fn next_batch(&self) -> Option<Vec<FromDaemon>> {
        loop {
            {
                let mut queue = self.queue();
                if queue.overflowed {
                    return None;
                }
                let mut batch: Vec<FromDaemon> = queue.messages.drain(..).collect();
                while let Some(id) = queue.changed.pop_front() {
                    if let Some(session) = queue.views.remove(&id) {
                        batch.push(FromDaemon::SessionChanged { session });
                    }
                }
                if !batch.is_empty() {
                    return Some(batch);
                }
            }
            self.ready.notified().await;
        }
    }
}
