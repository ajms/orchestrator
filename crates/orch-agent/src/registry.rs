use std::sync::Arc;

use crate::{AgentAdapter, CLAUDE, ClaudeCode};

pub type Adapter = Arc<dyn AgentAdapter + Send + Sync>;

pub fn adapter_named(name: &str, binary: Option<&str>) -> Option<Adapter> {
    match name {
        CLAUDE => {
            let mut claude = ClaudeCode::default();
            if let Some(binary) = binary {
                claude.program = binary.into();
            }
            Some(Arc::new(claude))
        }
        _ => None,
    }
}

pub fn implied_agent() -> String {
    CLAUDE.into()
}
