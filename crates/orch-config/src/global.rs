use std::path::{Path, PathBuf};

use std::time::Duration;

use serde::Deserialize;

use crate::error::ConfigProblem;
use crate::loader::expand_home;
use crate::notifications::{Notifications, NotificationsLayer};
use crate::ports::PortRange;
use crate::repo::{PersonalOverrides, RepoLayer};

#[derive(Debug, Clone, PartialEq)]
pub struct GlobalConfig {
    pub branch_prefix: String,
    pub stalled_after: Duration,
    pub ports: PortRange,
    pub notifications: Notifications,
    pub signing_key: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GlobalFile {
    branch_prefix: Option<String>,
    stalled_minutes: Option<u64>,
    ports: Option<PortRange>,
    #[serde(default)]
    pub(crate) notifications: NotificationsLayer,
    #[serde(default)]
    pub(crate) defaults: RepoLayer,
    #[serde(default)]
    pub(crate) repos: PersonalOverrides,
    signing: Option<SigningFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SigningFile {
    key: String,
}

impl GlobalFile {
    pub(crate) fn check(&self) -> Result<(), ConfigProblem> {
        let ports = self.ports.unwrap_or_default();
        if ports.blocks().next().is_none() {
            return Err(ConfigProblem::EmptyPortRange(ports));
        }
        if !self.defaults.notifications.is_empty() {
            return Err(ConfigProblem::Misplaced {
                key: "notifications",
            });
        }
        Ok(())
    }

    pub(crate) fn resolve(&self, home: Option<&Path>) -> GlobalConfig {
        GlobalConfig {
            branch_prefix: self.branch_prefix.clone().unwrap_or_else(|| "orch/".into()),
            stalled_after: Duration::from_secs(60 * self.stalled_minutes.unwrap_or(10)),
            ports: self.ports.unwrap_or_default(),
            notifications: Notifications::layered([&self.notifications]),
            signing_key: self
                .signing
                .as_ref()
                .map(|signing| expand_home(&signing.key, home)),
        }
    }
}
