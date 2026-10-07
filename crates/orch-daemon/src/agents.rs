use std::path::Path;
use std::sync::Arc;

use orch_agent::{AgentAdapter, ClaudeCode};
use orch_config::AgentConfig;

pub(crate) type Adapter = Arc<dyn AgentAdapter + Send + Sync>;

pub(crate) const BUILT_IN: [&str; 1] = ["claude"];

fn build(name: &str, binary: Option<&str>) -> Option<(Adapter, String)> {
    match name {
        "claude" => {
            let mut claude = ClaudeCode::default();
            if let Some(binary) = binary {
                claude.program = binary.into();
            }
            let program = claude.program.clone();
            Some((Arc::new(claude), program))
        }
        _ => None,
    }
}

fn unknown(name: &str) -> String {
    format!(
        "unknown Agent \"{name}\"; the built-in Agents are {}",
        BUILT_IN.join(", ")
    )
}

pub(crate) fn adapter_for(agent: &AgentConfig) -> Result<Adapter, String> {
    build(&agent.name, agent.binary.as_deref())
        .map(|(adapter, _)| adapter)
        .ok_or_else(|| unknown(&agent.name))
}

pub(crate) fn session_adapter(name: &str) -> Option<Adapter> {
    build(name, None).map(|(adapter, _)| adapter)
}

pub(crate) fn default_program(name: &str) -> String {
    build(name, None).map_or_else(|| name.into(), |(_, program)| program)
}

pub(crate) fn launchable(agent: &AgentConfig, cwd: &Path) -> Result<Adapter, String> {
    let (adapter, program) =
        build(&agent.name, agent.binary.as_deref()).ok_or_else(|| unknown(&agent.name))?;
    match installed(&program, cwd) {
        true => Ok(adapter),
        false => Err(format!(
            "the {} Agent's binary \"{program}\" is not installed",
            agent.name
        )),
    }
}

fn installed(program: &str, cwd: &Path) -> bool {
    if program.contains('/') {
        return cwd.join(program).is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}
