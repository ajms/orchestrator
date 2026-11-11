use orch_core::PermissionMode;

pub const INHERIT: &str = "inherit";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preset {
    pub name: String,
    pub mode: Option<PermissionMode>,
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    pub sandbox: Option<bool>,
}

impl Preset {
    pub fn inherit() -> Self {
        Self::built_in(INHERIT, None)
    }

    fn built_in(name: &str, mode: Option<PermissionMode>) -> Self {
        Self {
            name: name.into(),
            mode,
            allow: Vec::new(),
            deny: Vec::new(),
            sandbox: None,
        }
    }

    pub fn sandboxed(&self) -> bool {
        self.sandbox.unwrap_or(true)
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
        permissive_mode || !self.allow.is_empty() || self.sandbox == Some(false)
    }

    /// Whether a Child Session on this Preset could do more than its parent
    /// on `ceiling`: a more permissive mode, leaving the Sandbox, allowing a
    /// tool the parent doesn't, or dropping one of the parent's denials. A
    /// Preset that inherits the Agent's own mode counts as the most permissive,
    /// since the Orchestrator can't see what it inherits.
    pub fn loosens_beyond(&self, ceiling: &Preset) -> bool {
        let rank = |mode: Option<PermissionMode>| match mode {
            Some(PermissionMode::Plan | PermissionMode::DontAsk) => 0,
            Some(PermissionMode::Default) => 1,
            Some(PermissionMode::AcceptEdits) => 2,
            Some(PermissionMode::Auto) => 3,
            Some(PermissionMode::BypassPermissions) => 4,
            None => 5,
        };
        rank(self.mode) > rank(ceiling.mode)
            || (ceiling.sandboxed() && !self.sandboxed())
            || self.allow.iter().any(|tool| !ceiling.allow.contains(tool))
            || ceiling.deny.iter().any(|tool| !self.deny.contains(tool))
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

#[cfg(test)]
mod ceiling_tests {
    use super::*;

    fn preset(mode: Option<PermissionMode>) -> Preset {
        Preset::built_in("p", mode)
    }

    #[test]
    fn a_child_preset_may_be_as_strict_or_stricter_than_its_parent() {
        let ask = preset(Some(PermissionMode::Default));
        assert!(!preset(Some(PermissionMode::Plan)).loosens_beyond(&ask));
        assert!(!ask.loosens_beyond(&ask));
        assert!(preset(Some(PermissionMode::Auto)).loosens_beyond(&ask));
        assert!(preset(None).loosens_beyond(&ask));
        assert!(!preset(Some(PermissionMode::Auto)).loosens_beyond(&preset(None)));
    }

    #[test]
    fn leaving_the_sandbox_or_widening_tools_loosens() {
        let parent = Preset {
            deny: vec!["Bash(rm:*)".into()],
            ..preset(Some(PermissionMode::Default))
        };
        let unsandboxed = Preset {
            sandbox: Some(false),
            ..parent.clone()
        };
        let allowing = Preset {
            allow: vec!["Bash".into()],
            ..parent.clone()
        };
        let undenied = Preset {
            deny: Vec::new(),
            ..parent.clone()
        };
        assert!(unsandboxed.loosens_beyond(&parent));
        assert!(allowing.loosens_beyond(&parent));
        assert!(undenied.loosens_beyond(&parent));
    }
}
