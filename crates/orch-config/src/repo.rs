use std::collections::BTreeMap;

use orch_agent::{INHERIT, Preset, Presets, mode_from_name};
use serde::Deserialize;

use crate::error::ConfigProblem;
use crate::notifications::{Notifications, NotificationsLayer};
use crate::trust::{TrustHash, TrustItem, TrustRequest, Untrusted};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Source {
    GlobalDefault,
    RepoFile,
    PersonalOverride,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Sourced<T> {
    value: T,
    source: Source,
}

impl<T> Sourced<T> {
    fn committed(&self) -> bool {
        self.source == Source::RepoFile
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RepoLayer {
    setup: Option<String>,
    teardown: Option<String>,
    base: Option<String>,
    preset: Option<String>,
    review_command: Option<String>,
    #[serde(default)]
    agent: AgentLayer,
    #[serde(default)]
    pub(crate) notifications: NotificationsLayer,
    #[serde(default)]
    presets: BTreeMap<String, PresetLayer>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentLayer {
    name: Option<String>,
    binary: Option<String>,
    args: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PresetLayer {
    mode: Option<String>,
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default)]
    deny: Vec<String>,
}

impl PresetLayer {
    fn to_preset(&self, name: &str) -> Result<Preset, ConfigProblem> {
        let mode =
            match self.mode.as_deref() {
                None | Some(INHERIT) => None,
                Some(mode) => Some(mode_from_name(mode).ok_or_else(|| {
                    ConfigProblem::UnknownPermissionMode {
                        preset: name.into(),
                        mode: mode.into(),
                    }
                })?),
            };
        Ok(Preset {
            name: name.into(),
            mode,
            allow: self.allow.clone(),
            deny: self.deny.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentConfig {
    pub name: String,
    pub binary: Option<String>,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresetError {
    Unknown(String),
    Untrusted(String),
}

#[derive(Debug, Clone)]
pub struct RepoConfig {
    setup: Option<Sourced<String>>,
    teardown: Option<Sourced<String>>,
    base: Option<String>,
    preset: Option<Sourced<String>>,
    review_command: Option<String>,
    agent: AgentConfig,
    agent_committed: bool,
    notifications: Notifications,
    presets: BTreeMap<String, Sourced<Preset>>,
    trust_request: Option<TrustRequest>,
    trusted: bool,
}

impl RepoConfig {
    pub(crate) fn layered(
        global_notifications: &NotificationsLayer,
        layers: [(Source, &RepoLayer); 3],
        approved: Option<&TrustHash>,
    ) -> Result<Self, (Source, ConfigProblem)> {
        for (source, layer) in &layers {
            if *source == Source::RepoFile && layer.review_command.is_some() {
                let key = "review_command";
                return Err((*source, ConfigProblem::Misplaced { key }));
            }
        }
        let pick = |get: &dyn Fn(&RepoLayer) -> Option<&String>| {
            layers.iter().rev().find_map(|(source, layer)| {
                get(layer).map(|value| Sourced {
                    value: value.clone(),
                    source: *source,
                })
            })
        };
        let binary = pick(&|layer| layer.agent.binary.as_ref());
        let args = layers.iter().rev().find_map(|(source, layer)| {
            layer.agent.args.clone().map(|value| Sourced {
                value,
                source: *source,
            })
        });
        let agent_committed = binary.as_ref().is_some_and(Sourced::committed)
            || args.as_ref().is_some_and(Sourced::committed);
        let agent = AgentConfig {
            name: pick(&|layer| layer.agent.name.as_ref())
                .map_or_else(|| "claude".into(), |name| name.value),
            binary: binary.map(|binary| binary.value),
            args: args.map(|args| args.value).unwrap_or_default(),
        };

        let mut presets = BTreeMap::new();
        for (source, layer) in &layers {
            for (name, preset) in &layer.presets {
                if name == INHERIT {
                    return Err((*source, ConfigProblem::ReservedPresetName));
                }
                let value = preset.to_preset(name).map_err(|err| (*source, err))?;
                presets.insert(
                    name.clone(),
                    Sourced {
                        value,
                        source: *source,
                    },
                );
            }
        }

        let mut config = Self {
            setup: pick(&|layer| layer.setup.as_ref()),
            teardown: pick(&|layer| layer.teardown.as_ref()),
            base: pick(&|layer| layer.base.as_ref()).map(|sourced| sourced.value),
            preset: pick(&|layer| layer.preset.as_ref()),
            review_command: pick(&|layer| layer.review_command.as_ref())
                .map(|sourced| sourced.value),
            agent,
            agent_committed,
            notifications: Notifications::layered(
                std::iter::once(global_notifications)
                    .chain(layers.iter().map(|(_, layer)| &layer.notifications)),
            ),
            presets,
            trust_request: None,
            trusted: false,
        };
        config.trust_request = TrustRequest::for_items(config.trust_items());
        config.trusted = match &config.trust_request {
            None => true,
            Some(request) => approved == Some(&request.hash),
        };
        Ok(config)
    }

    fn trust_items(&self) -> Vec<TrustItem> {
        let committed = |sourced: &Option<Sourced<String>>| {
            sourced
                .as_ref()
                .filter(|sourced| sourced.committed())
                .map(|sourced| sourced.value.clone())
        };
        let mut items = Vec::new();
        items.extend(committed(&self.setup).map(TrustItem::SetupScript));
        items.extend(committed(&self.teardown).map(TrustItem::TeardownScript));
        if self.agent_committed {
            items.push(TrustItem::Agent {
                binary: self.agent.binary.clone(),
                args: self.agent.args.clone(),
            });
        }
        items.extend(
            self.presets
                .values()
                .filter(|preset| preset.committed() && preset.value.loosens())
                .map(|preset| TrustItem::Preset(preset.value.clone())),
        );
        items.extend(
            self.committed_loosening_default()
                .map(TrustItem::DefaultPreset),
        );
        items
    }

    fn committed_loosening_default(&self) -> Option<Preset> {
        let name = self.preset.as_ref().filter(|preset| preset.committed())?;
        self.all_presets()
            .get(&name.value)
            .filter(|preset| preset.loosens())
            .cloned()
    }

    fn all_presets(&self) -> Presets {
        let all = self.presets.values().map(|preset| preset.value.clone());
        Presets::new(all.collect()).expect("reserved preset names are rejected at load")
    }

    fn usable<'a, T>(&self, sourced: &'a Option<Sourced<T>>) -> Result<Option<&'a T>, Untrusted> {
        match sourced {
            Some(sourced) if sourced.committed() && !self.trusted => Err(Untrusted),
            sourced => Ok(sourced.as_ref().map(|sourced| &sourced.value)),
        }
    }

    fn preset_usable(&self, preset: &Sourced<Preset>) -> bool {
        self.trusted || !(preset.committed() && preset.value.loosens())
    }

    pub fn trust_request(&self) -> Option<TrustRequest> {
        self.trust_request.clone()
    }

    pub fn is_trusted(&self) -> bool {
        self.trusted
    }

    pub fn setup_script(&self) -> Result<Option<&str>, Untrusted> {
        self.usable(&self.setup)
            .map(|script| script.map(String::as_str))
    }

    pub fn teardown_script(&self) -> Result<Option<&str>, Untrusted> {
        self.usable(&self.teardown)
            .map(|script| script.map(String::as_str))
    }

    pub fn review_command(&self) -> Option<&str> {
        self.review_command.as_deref()
    }

    pub fn agent(&self) -> Result<&AgentConfig, Untrusted> {
        match self.agent_committed && !self.trusted {
            true => Err(Untrusted),
            false => Ok(&self.agent),
        }
    }

    pub fn base_branch(&self) -> Option<&str> {
        self.base.as_deref()
    }

    pub fn default_preset(&self) -> Option<&str> {
        self.preset.as_ref().map(|preset| preset.value.as_str())
    }

    pub fn notifications(&self) -> &Notifications {
        &self.notifications
    }

    pub fn presets(&self) -> Presets {
        let usable = self
            .presets
            .values()
            .filter(|preset| self.preset_usable(preset))
            .map(|preset| preset.value.clone())
            .collect();
        Presets::new(usable).expect("reserved preset names are rejected at load")
    }

    pub fn select_preset(&self, new: Option<&str>) -> Result<Preset, PresetError> {
        let name = new.or(self.default_preset()).unwrap_or(INHERIT);
        let untrusted_definition = self
            .presets
            .get(name)
            .is_some_and(|preset| !self.preset_usable(preset));
        let untrusted_default =
            new.is_none() && !self.trusted && self.committed_loosening_default().is_some();
        if untrusted_definition || untrusted_default {
            return Err(PresetError::Untrusted(name.into()));
        }
        self.presets()
            .get(name)
            .cloned()
            .ok_or_else(|| PresetError::Unknown(name.into()))
    }
}

pub(crate) type PersonalOverrides = BTreeMap<String, RepoLayer>;
