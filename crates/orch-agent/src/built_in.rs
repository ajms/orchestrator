use std::sync::Arc;

use crate::{AgentAdapter, Antigravity, ClaudeCode};

pub type Adapter = Arc<dyn AgentAdapter + Send + Sync>;

type Build = fn(Option<String>) -> (Adapter, String);

const BUILT_IN: [(&str, Build); 2] = [
    (ClaudeCode::NAME, build::<ClaudeCode>),
    (Antigravity::NAME, build::<Antigravity>),
];

trait BuiltIn: AgentAdapter + Default + Send + Sync + 'static {
    fn program(&mut self) -> &mut String;
}

impl BuiltIn for ClaudeCode {
    fn program(&mut self) -> &mut String {
        &mut self.program
    }
}

impl BuiltIn for Antigravity {
    fn program(&mut self) -> &mut String {
        &mut self.program
    }
}

fn build<A: BuiltIn>(binary: Option<String>) -> (Adapter, String) {
    let mut adapter = A::default();
    if let Some(binary) = binary {
        *adapter.program() = binary;
    }
    let program = adapter.program().clone();
    (Arc::new(adapter), program)
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
