use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use orch_core::SessionId;

use crate::policy::{DesktopAction, Notification, NotificationKey};
use crate::sink::NotificationSink;

pub const APP_NAME: &str = "orch";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NotificationId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActionKey(pub String);

impl ActionKey {
    pub const DEFAULT: &str = "default";

    pub fn default_action() -> Self {
        Self(Self::DEFAULT.into())
    }

    pub fn is_default(&self) -> bool {
        self.0 == Self::DEFAULT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpireTimeout {
    ServerDefault,
    Never,
    After(Duration),
}

impl ExpireTimeout {
    pub fn as_millis(self) -> i32 {
        match self {
            Self::ServerDefault => -1,
            Self::Never => 0,
            Self::After(duration) => i32::try_from(duration.as_millis()).unwrap_or(i32::MAX),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyCall {
    pub app_name: String,
    pub replaces: Option<NotificationId>,
    pub app_icon: String,
    pub title: String,
    pub body: String,
    pub actions: Vec<(ActionKey, String)>,
    pub expire: ExpireTimeout,
}

impl NotifyCall {
    fn for_notification(notification: &Notification, replaces: Option<NotificationId>) -> Self {
        Self {
            app_name: APP_NAME.into(),
            replaces,
            app_icon: String::new(),
            title: notification.title.clone(),
            body: escape_markup(&notification.body),
            actions: vec![(ActionKey::default_action(), "Open".into())],
            expire: ExpireTimeout::ServerDefault,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusError(pub String);

impl fmt::Display for BusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for BusError {}

pub trait Bus {
    fn notify(&self, call: &NotifyCall) -> Result<NotificationId, BusError>;
    fn close(&self, id: NotificationId) -> Result<(), BusError>;
}

#[derive(Debug, Clone, Default)]
pub struct ClickRoutes(Arc<Mutex<HashMap<NotificationId, SessionId>>>);

impl ClickRoutes {
    pub fn route(&self, id: NotificationId, action: &ActionKey) -> Option<SessionId> {
        if !action.is_default() {
            return None;
        }
        self.routes().get(&id).cloned()
    }

    fn routes(&self) -> MutexGuard<'_, HashMap<NotificationId, SessionId>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub struct DesktopSink<B> {
    bus: B,
    ids: HashMap<NotificationKey, NotificationId>,
    clicks: ClickRoutes,
}

impl<B: Bus> DesktopSink<B> {
    pub fn new(bus: B) -> Self {
        Self {
            bus,
            ids: HashMap::new(),
            clicks: ClickRoutes::default(),
        }
    }

    pub fn clicks(&self) -> ClickRoutes {
        self.clicks.clone()
    }

    fn forget(&mut self, key: &NotificationKey) -> Option<NotificationId> {
        let id = self.ids.remove(key)?;
        self.clicks.routes().remove(&id);
        Some(id)
    }
}

impl<B: Bus> NotificationSink for DesktopSink<B> {
    fn apply(&mut self, action: &DesktopAction) {
        match action {
            DesktopAction::Show { key, notification } => {
                let replaces = self.forget(key);
                match self
                    .bus
                    .notify(&NotifyCall::for_notification(notification, replaces))
                {
                    Ok(id) => {
                        self.ids.insert(key.clone(), id);
                        self.clicks.routes().insert(id, notification.focus.clone());
                    }
                    Err(err) => eprintln!("orch-notify: desktop notification dropped: {err}"),
                }
            }
            DesktopAction::Close { key } => {
                if let Some(id) = self.forget(key) {
                    let _ = self.bus.close(id);
                }
            }
        }
    }
}

fn escape_markup(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
