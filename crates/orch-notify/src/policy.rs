use std::collections::HashMap;
use std::time::{Duration, Instant};

use orch_config::{Channel, Notifications};
use orch_core::{Attention, SessionId};

pub const MERGE_WINDOW: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionEvent {
    pub session: SessionId,
    pub title: String,
    pub branch: String,
    pub attention: Attention,
    pub at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientView {
    pub id: ClientId,
    pub focused: bool,
    pub showing: Option<SessionId>,
}

#[derive(Debug, Clone, Copy)]
pub struct AttentionContext<'a> {
    pub muted: bool,
    pub notifications: &'a Notifications,
    pub clients: &'a [ClientView],
}

impl AttentionContext<'_> {
    fn is_looking_at(&self, session: &SessionId) -> bool {
        self.clients
            .iter()
            .any(|client| client.focused && client.showing.as_ref() == Some(session))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NotificationKey {
    Session(SessionId),
    Merged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub title: String,
    pub body: String,
    pub focus: SessionId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopAction {
    Show {
        key: NotificationKey,
        notification: Notification,
    },
    Close {
        key: NotificationKey,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ring {
    pub client: ClientId,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Actions {
    pub desktop: Vec<DesktopAction>,
    pub bell: Vec<Ring>,
}

#[derive(Debug, Default)]
pub struct NotificationPolicy {
    burst: Vec<SessionId>,
    last_shown: Option<Instant>,
    alone: HashMap<SessionId, Callout>,
    merged: Vec<Callout>,
}

#[derive(Debug, Clone)]
struct Callout {
    session: SessionId,
    title: String,
    attention: Attention,
    body: String,
}

impl Callout {
    fn shown_alone(&self) -> DesktopAction {
        show(
            NotificationKey::Session(self.session.clone()),
            self.title.clone(),
            self.body.clone(),
            self.session.clone(),
        )
    }
}

impl NotificationPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_attention(
        &mut self,
        event: AttentionEvent,
        context: AttentionContext<'_>,
    ) -> Actions {
        if context.muted || context.is_looking_at(&event.session) {
            return Actions::default();
        }
        let callout = Callout {
            body: format!("{} · {}", label(event.attention), event.branch),
            session: event.session,
            title: event.title,
            attention: event.attention,
        };
        let enabled = |channel| context.notifications.enabled(channel, callout.attention);
        let mut actions = Actions::default();
        if enabled(Channel::Bell) {
            actions.bell = context
                .clients
                .iter()
                .map(|client| Ring {
                    client: client.id,
                    title: callout.title.clone(),
                    body: callout.body.clone(),
                })
                .collect();
        }
        if enabled(Channel::Desktop) {
            actions.desktop = self.call_out(callout, event.at);
        }
        actions
    }

    pub fn dismiss(&mut self, session: &SessionId) -> Vec<DesktopAction> {
        self.burst.retain(|member| member != session);
        if self.alone.remove(session).is_some() {
            return vec![close(NotificationKey::Session(session.clone()))];
        }
        let Some(position) = self.merged.iter().position(|c| &c.session == session) else {
            return Vec::new();
        };
        self.merged.remove(position);
        match self.merged.as_slice() {
            [] => vec![close(NotificationKey::Merged)],
            [_] => {
                let last = self.merged.remove(0);
                let action = last.shown_alone();
                self.alone.insert(last.session.clone(), last);
                vec![close(NotificationKey::Merged), action]
            }
            [.., latest] => vec![self.merged_notification(latest.session.clone())],
        }
    }

    fn call_out(&mut self, callout: Callout, at: Instant) -> Vec<DesktopAction> {
        let within_window = self
            .last_shown
            .is_some_and(|last| at.saturating_duration_since(last) < MERGE_WINDOW);
        if !within_window {
            self.burst.clear();
        }
        self.last_shown = Some(at);
        if !self.burst.contains(&callout.session) {
            self.burst.push(callout.session.clone());
        }

        let already_merged = self.merged.iter().any(|c| c.session == callout.session);
        if self.burst.len() == 1 && !already_merged {
            let action = callout.shown_alone();
            self.alone.insert(callout.session.clone(), callout);
            return vec![action];
        }

        let mut actions = Vec::new();
        for session in &self.burst {
            if let Some(merging) = self.alone.remove(session) {
                actions.push(close(NotificationKey::Session(session.clone())));
                upsert(&mut self.merged, merging);
            }
        }
        let focus = callout.session.clone();
        upsert(&mut self.merged, callout);
        actions.push(self.merged_notification(focus));
        actions
    }

    fn merged_notification(&self, focus: SessionId) -> DesktopAction {
        let count = self.merged.len();
        let body = self
            .merged
            .iter()
            .map(|callout| format!("{}: {}", callout.title, label(callout.attention)))
            .collect::<Vec<_>>()
            .join("\n");
        show(
            NotificationKey::Merged,
            format!("{count} Sessions need you"),
            body,
            focus,
        )
    }
}

fn show(key: NotificationKey, title: String, body: String, focus: SessionId) -> DesktopAction {
    DesktopAction::Show {
        key,
        notification: Notification { title, body, focus },
    }
}

fn close(key: NotificationKey) -> DesktopAction {
    DesktopAction::Close { key }
}

fn upsert(callouts: &mut Vec<Callout>, callout: Callout) {
    match callouts.iter_mut().find(|c| c.session == callout.session) {
        Some(existing) => *existing = callout,
        None => callouts.push(callout),
    }
}

fn label(attention: Attention) -> &'static str {
    match attention {
        Attention::NeedsInput => "Needs input",
        Attention::TurnEnded => "Finished its turn",
        Attention::Errored => "Errored",
        Attention::SetupFailed => "Setup failed",
        Attention::ChecksFailing => "Checks failing",
        Attention::ChangesRequested => "Changes requested",
        Attention::PrMerged => "PR merged",
        Attention::PrClosed => "PR closed",
    }
}
