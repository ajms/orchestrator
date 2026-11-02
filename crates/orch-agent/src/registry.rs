use std::sync::Arc;

use crate::{AgentAdapter, CLAUDE, ClaudeCode, PI, Pi};

pub type Adapter = Arc<dyn AgentAdapter + Send + Sync>;

pub fn adapter_named(name: &str, binary: Option<&str>) -> Option<Adapter> {
    let program = |default: String| binary.map_or(default, String::from);
    match name {
        CLAUDE => Some(Arc::new(ClaudeCode {
            program: program(ClaudeCode::default().program),
        })),
        PI => Some(Arc::new(Pi {
            program: program(Pi::default().program),
        })),
        _ => None,
    }
}

pub fn implied_agent() -> String {
    CLAUDE.into()
}
