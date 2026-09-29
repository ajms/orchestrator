#![allow(dead_code)]

use orch_core::SessionId;
use orch_notify::{DesktopAction, Notification, NotificationKey};

pub fn session(id: &str) -> SessionId {
    SessionId::parse(id).unwrap()
}

pub fn notification(title: &str, body: &str, focus: &str) -> Notification {
    Notification {
        title: title.into(),
        body: body.into(),
        focus: session(focus),
    }
}

pub fn show_session(id: &str, title: &str, body: &str) -> DesktopAction {
    DesktopAction::Show {
        key: NotificationKey::Session(session(id)),
        notification: notification(title, body, id),
    }
}

pub fn show_merged(title: &str, body: &str, focus: &str) -> DesktopAction {
    DesktopAction::Show {
        key: NotificationKey::Merged,
        notification: notification(title, body, focus),
    }
}

pub fn close_session(id: &str) -> DesktopAction {
    DesktopAction::Close {
        key: NotificationKey::Session(session(id)),
    }
}

pub fn close_merged() -> DesktopAction {
    DesktopAction::Close {
        key: NotificationKey::Merged,
    }
}
