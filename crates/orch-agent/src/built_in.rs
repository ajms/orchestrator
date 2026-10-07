use std::sync::Arc;

use crate::{AgentAdapter, Antigravity, ClaudeCode};

pub type Adapter = Arc<dyn AgentAdapter + Send + Sync>;

type Build = fn(Option<String>) -> (Adapter, String);

const BUILT_IN: [(&str, Build); 2] = [(ClaudeCode::NAME, claude), (Antigravity::NAME, antigravity)];

fn claude(binary: Option<String>) -> (Adapter, String) {
    let mut claude = ClaudeCode::default();
    if let Some(binary) = binary {
        claude.program = binary;
    }
    let program = claude.program.clone();
    (Arc::new(claude), program)
}

fn antigravity(binary: Option<String>) -> (Adapter, String) {
    let mut antigravity = Antigravity::default();
    if let Some(binary) = binary {
        antigravity.program = binary;
    }
    let program = antigravity.program.clone();
    (Arc::new(antigravity), program)
}

pub fn built_in_names() -> impl Iterator<Item = &'static str> {
    BUILT_IN.iter().map(|(name, _)| *name)
}

pub fn by_name(name: &str) -> Option<Adapter> {
    with_binary(name, None).map(|(adapter, _)| adapter)
}

pub fn with_binary(name: &str, binary: Option<String>) -> Option<(Adapter, String)> {
    BUILT_IN
        .iter()
        .find(|(built_in, _)| *built_in == name)
        .map(|(_, build)| build(binary))
}
