use std::sync::Arc;

use orch_agent::{AgentAdapter, CLAUDE, ClaudeCode};
use orch_config::AgentConfig;

pub(crate) type Adapter = Arc<dyn AgentAdapter + Send + Sync>;

pub(crate) fn adapter_for(agent: &AgentConfig) -> Result<Adapter, String> {
    match agent.name.as_str() {
        CLAUDE => {
            let mut claude = ClaudeCode::default();
            if let Some(binary) = &agent.binary {
                claude.program = binary.clone();
            }
            Ok(Arc::new(claude))
        }
        other => Err(format!(
            "unknown Agent \"{other}\"; only claude is supported"
        )),
    }
}

pub(crate) fn session_adapter(agent: &str) -> Option<Adapter> {
    let agent = AgentConfig {
        name: agent.into(),
        binary: None,
        args: Vec::new(),
    };
    adapter_for(&agent).ok()
}

pub(crate) fn default_adapter() -> Adapter {
    Arc::new(ClaudeCode::default())
}

pub(crate) fn default_program(agent: &str) -> String {
    match agent {
        CLAUDE => ClaudeCode::default().program,
        other => other.into(),
    }
}
