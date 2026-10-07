use std::path::Path;
use std::sync::Arc;

use orch_agent::{AgentAdapter, ClaudeCode};
use orch_config::AgentConfig;

pub(crate) type Adapter = Arc<dyn AgentAdapter + Send + Sync>;

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

pub(crate) fn built_in_names() -> impl Iterator<Item = &'static str> {
    BUILT_IN.iter().map(|(name, _)| *name)
}

fn build(name: &str, binary: Option<String>) -> Option<(Adapter, String)> {
    BUILT_IN
        .iter()
        .find(|(built_in, _)| *built_in == name)
        .map(|(_, build)| build(binary))
}

fn unknown(name: &str) -> String {
    let names = built_in_names().collect::<Vec<_>>();
    format!(
        "unknown Agent \"{name}\"; the built-in Agents are {}",
        names.join(", ")
    )
}

pub(crate) fn known_adapter(agent: &AgentConfig) -> Result<Adapter, String> {
    build(&agent.name, agent.binary.clone())
        .map(|(adapter, _)| adapter)
        .ok_or_else(|| unknown(&agent.name))
}

pub(crate) fn adapter_by_name(name: &str) -> Option<Adapter> {
    build(name, None).map(|(adapter, _)| adapter)
}

pub(crate) fn default_program(name: &str) -> String {
    build(name, None).map_or_else(|| name.into(), |(_, program)| program)
}

pub(crate) fn installed_adapter(agent: &AgentConfig, repo: &Path) -> Result<Adapter, String> {
    let binary = agent
        .binary
        .as_deref()
        .map(|binary| match binary.contains('/') {
            true => repo.join(binary).to_string_lossy().into_owned(),
            false => binary.into(),
        });
    let (adapter, program) = build(&agent.name, binary).ok_or_else(|| unknown(&agent.name))?;
    match installed(&program) {
        true => Ok(adapter),
        false => Err(format!(
            "the {} Agent's binary \"{program}\" is not installed",
            agent.name
        )),
    }
}

fn installed(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}
