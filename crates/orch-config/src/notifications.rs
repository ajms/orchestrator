use std::collections::BTreeMap;

use orch_core::Attention;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    Desktop,
    Bell,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NotificationsLayer {
    #[serde(default)]
    desktop: BTreeMap<Attention, bool>,
    #[serde(default)]
    bell: BTreeMap<Attention, bool>,
}

impl NotificationsLayer {
    pub(crate) fn is_empty(&self) -> bool {
        self.desktop.is_empty() && self.bell.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Notifications {
    desktop: BTreeMap<Attention, bool>,
    bell: BTreeMap<Attention, bool>,
}

impl Notifications {
    pub(crate) fn layered<'a>(layers: impl IntoIterator<Item = &'a NotificationsLayer>) -> Self {
        let mut merged = Self::default();
        for layer in layers {
            merged.desktop.extend(&layer.desktop);
            merged.bell.extend(&layer.bell);
        }
        merged
    }

    pub fn enabled(&self, channel: Channel, attention: Attention) -> bool {
        let triggers = match channel {
            Channel::Desktop => &self.desktop,
            Channel::Bell => &self.bell,
        };
        triggers.get(&attention).copied().unwrap_or(true)
    }
}
