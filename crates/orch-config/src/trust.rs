use std::fmt;

use orch_agent::{Preset, mode_name};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrustHash(String);

impl TrustHash {
    pub fn from_stored(hex: impl Into<String>) -> Self {
        Self(hex.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TrustHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustItem {
    SetupScript(String),
    TeardownScript(String),
    Agent {
        name: String,
        binary: Option<String>,
        args: Vec<String>,
    },
    Preset(Preset),
    DefaultPreset(Preset),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustRequest {
    pub hash: TrustHash,
    pub items: Vec<TrustItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Untrusted;

impl fmt::Display for Untrusted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the Repo's committed config is not trusted")
    }
}

impl std::error::Error for Untrusted {}

impl TrustRequest {
    pub(crate) fn for_items(items: Vec<TrustItem>) -> Option<Self> {
        if items.is_empty() {
            return None;
        }
        let mut hasher = Sha256::new();
        for item in &items {
            for field in fields(item) {
                hasher.update((field.len() as u64).to_le_bytes());
                hasher.update(field.as_bytes());
            }
        }
        let hash = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Some(Self {
            hash: TrustHash(hash),
            items,
        })
    }
}

fn fields(item: &TrustItem) -> Vec<String> {
    let list = |tag: &str, values: &[String]| {
        let mut fields = vec![tag.to_string(), values.len().to_string()];
        fields.extend(values.iter().cloned());
        fields
    };
    match item {
        TrustItem::SetupScript(script) => vec!["setup".into(), script.clone()],
        TrustItem::TeardownScript(script) => vec!["teardown".into(), script.clone()],
        TrustItem::Agent { name, binary, args } => {
            let mut fields = vec!["agent".into(), name.clone()];
            fields.extend(list("binary", binary.as_slice()));
            fields.extend(list("args", args));
            fields
        }
        TrustItem::Preset(preset) | TrustItem::DefaultPreset(preset) => {
            let tag = match item {
                TrustItem::DefaultPreset(_) => "default_preset",
                _ => "preset",
            };
            let mode = preset.mode.map_or("inherit", mode_name);
            let mut fields = vec![tag.into(), preset.name.clone(), mode.into()];
            fields.extend(list("allow", &preset.allow));
            fields.extend(list("deny", &preset.deny));
            fields
        }
    }
}
