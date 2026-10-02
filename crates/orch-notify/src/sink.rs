use orch_core::SessionId;

use crate::policy::{DesktopAction, Notification, NotificationKey};

pub trait NotificationSink {
    fn apply(&mut self, action: &DesktopAction);
}

#[derive(Debug, Default)]
pub struct RecordingSink {
    visible: Vec<(NotificationKey, Notification)>,
}

impl RecordingSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn visible(&self) -> Vec<(&NotificationKey, &Notification)> {
        self.visible.iter().map(|(key, n)| (key, n)).collect()
    }

    pub fn click(&self, key: &NotificationKey) -> Option<SessionId> {
        self.visible
            .iter()
            .find(|(shown, _)| shown == key)
            .map(|(_, notification)| notification.focus.clone())
    }
}

impl NotificationSink for RecordingSink {
    fn apply(&mut self, action: &DesktopAction) {
        let (key, shown) = match action {
            DesktopAction::Show { key, notification } => (key, Some(notification)),
            DesktopAction::Close { key } => (key, None),
        };
        self.visible.retain(|(visible, _)| visible != key);
        if let Some(notification) = shown {
            self.visible.push((key.clone(), notification.clone()));
        }
    }
}
