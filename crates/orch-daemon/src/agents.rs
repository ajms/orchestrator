use std::sync::Arc;

pub(crate) use orch_agent::Adapter;
use orch_agent::{ClaudeCode, adapter_named};
use orch_config::AgentConfig;

pub(crate) fn adapter_for(agent: &AgentConfig) -> Result<Adapter, String> {
    adapter_named(&agent.name, agent.binary.as_deref())
        .ok_or_else(|| format!("unknown Agent \"{}\"", agent.name))
}

pub(crate) fn default_adapter() -> Adapter {
    Arc::new(ClaudeCode::default())
}
