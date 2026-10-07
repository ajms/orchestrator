use std::sync::Arc;

use crate::{AgentAdapter, ClaudeCode};

pub type Adapter = Arc<dyn AgentAdapter + Send + Sync>;

type Build = fn(Option<String>) -> (Adapter, String);

const BUILT_IN: [(&str, Build); 1] = [(ClaudeCode::NAME, claude)];

fn claude(binary: Option<String>) -> (Adapter, String) {
    let mut claude = ClaudeCode::default();
    if let Some(binary) = binary {
        claude.program = binary;
    }
    let program = claude.program.clone();
    (Arc::new(claude), program)
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
