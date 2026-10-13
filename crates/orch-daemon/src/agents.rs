use std::sync::Arc;

use orch_agent::{AgentAdapter, ClaudeCode};
use orch_config::AgentConfig;

pub(crate) type Adapter = Arc<dyn AgentAdapter + Send + Sync>;

pub(crate) fn adapter_for(agent: &AgentConfig) -> Result<Adapter, String> {
    match agent.name.as_str() {
        "claude" => {
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

pub(crate) fn default_adapter() -> Adapter {
    Arc::new(ClaudeCode::default())
}

pub(crate) fn default_program() -> String {
    ClaudeCode::default().program
}
