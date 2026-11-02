use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

pub(crate) use orch_agent::Adapter;
use orch_agent::{ClaudeCode, adapter_named};
use orch_config::AgentConfig;

use crate::subprocess::{command, run};

const ENV_COMMAND_LIMIT: Duration = Duration::from_secs(60);

pub(crate) fn adapter_for(agent: &AgentConfig) -> Result<Adapter, String> {
    adapter_named(&agent.name, agent.binary.as_deref())
        .ok_or_else(|| format!("unknown Agent \"{}\"", agent.name))
}

pub(crate) fn default_adapter() -> Adapter {
    Arc::new(ClaudeCode::default())
}

pub(crate) async fn resolve_env(
    commands: &BTreeMap<String, String>,
    cwd: &Path,
) -> Result<Vec<(String, String)>, String> {
    let mut env = Vec::with_capacity(commands.len());
    for (key, script) in commands {
        let mut shell = command("sh");
        shell.arg("-c").arg(script).current_dir(cwd);
        let what = format!("the command for {key}");
        let value = run(shell, None, ENV_COMMAND_LIMIT, &what).await?;
        env.push((
            key.clone(),
            value.trim_end_matches(['\n', '\r']).to_string(),
        ));
    }
    Ok(env)
}
