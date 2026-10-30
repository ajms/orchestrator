use std::sync::Arc;

use crate::{AgentAdapter, CLAUDE, ClaudeCode, PI, Pi};

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
        PI => {
            let mut pi = Pi::default();
            if let Some(binary) = binary {
                pi.program = binary.into();
            }
            Some(Arc::new(pi))
        }
        _ => None,
    }
}

pub fn implied_agent() -> String {
    CLAUDE.into()
}
