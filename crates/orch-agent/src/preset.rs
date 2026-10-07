use std::collections::BTreeMap;

use orch_core::PermissionMode;

pub const INHERIT: &str = "inherit";
pub const EDITS: &str = "edits";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

impl Rules {
    pub fn is_empty(&self) -> bool {
        self.allow.is_empty() && self.deny.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleVerdict {
    Allow { rule: String },
    Deny { rule: String },
    Unverifiable,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preset {
    pub name: String,
    pub mode: Option<PermissionMode>,
    pub rules: BTreeMap<String, Rules>,
}

impl Preset {
    pub fn inherit() -> Self {
        Self::built_in(INHERIT, None)
    }

    fn built_in(name: &str, mode: Option<PermissionMode>) -> Self {
        Self {
            name: name.into(),
            mode,
            ..Self::default()
        }
    }

    pub fn rules_for(&self, agent: &str) -> Option<&Rules> {
        self.rules.get(agent)
    }

    pub fn loosens(&self) -> bool {
        let permissive_mode = matches!(
            self.mode,
            Some(
                PermissionMode::AcceptEdits
                    | PermissionMode::Auto
                    | PermissionMode::BypassPermissions
            )
        );
        permissive_mode || self.rules.values().any(|rules| !rules.allow.is_empty())
    }

    pub fn expressible(&self, modes: &[PermissionMode]) -> bool {
        self.mode.is_none_or(|mode| modes.contains(&mode))
    }

    pub fn lacks_rules(&self, agent: &str) -> bool {
        let has_rules = |rules: &Rules| !rules.is_empty();
        self.rules.values().any(has_rules) && !self.rules_for(agent).is_some_and(has_rules)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PresetSelection<'a> {
    pub new: Option<&'a str>,
    pub repo_default: Option<&'a str>,
    pub global_default: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownPreset(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReservedPresetName(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presets {
    presets: Vec<Preset>,
}

impl Default for Presets {
    fn default() -> Self {
        Self::with_built_ins(Vec::new())
    }
}

impl Presets {
    pub fn new(user_defined: Vec<Preset>) -> Result<Self, ReservedPresetName> {
        match user_defined.iter().find(|preset| preset.name == INHERIT) {
            Some(reserved) => Err(ReservedPresetName(reserved.name.clone())),
            None => Ok(Self::with_built_ins(user_defined)),
        }
    }

    fn with_built_ins(user_defined: Vec<Preset>) -> Self {
        let built_ins = [
            Preset::built_in("plan", Some(PermissionMode::Plan)),
            Preset::built_in("ask", Some(PermissionMode::Default)),
            Preset::built_in("edits", Some(PermissionMode::AcceptEdits)),
            Preset::built_in("auto", Some(PermissionMode::Auto)),
            Preset::inherit(),
        ];
        let mut presets = user_defined;
        for built_in in built_ins {
            if !presets.iter().any(|preset| preset.name == built_in.name) {
                presets.push(built_in);
            }
        }
        Self { presets }
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.presets.iter().map(|preset| preset.name.as_str())
    }

    pub fn offered<'a>(
        &'a self,
        modes: &'a [PermissionMode],
    ) -> impl Iterator<Item = &'a Preset> + 'a {
        self.presets
            .iter()
            .filter(|preset| preset.expressible(modes))
    }

    pub fn get(&self, name: &str) -> Option<&Preset> {
        self.presets.iter().find(|preset| preset.name == name)
    }

    pub fn select(&self, selection: PresetSelection) -> Result<Preset, UnknownPreset> {
        let name = selection
            .new
            .or(selection.repo_default)
            .or(selection.global_default)
            .unwrap_or(INHERIT);
        self.get(name)
            .cloned()
            .ok_or_else(|| UnknownPreset(name.into()))
    }
}
