use std::path::Path;

use orch_agent::{Adapter, HookupState, built_in_names, by_name, with_binary};
use orch_config::{AgentConfig, xdg};
use orch_core::PermissionMode;

fn unknown(name: &str) -> String {
    let names = built_in_names().collect::<Vec<_>>();
    format!(
        "unknown Agent \"{name}\"; the built-in Agents are {}",
        names.join(", ")
    )
}

pub(crate) fn known_adapter(agent: &AgentConfig) -> Result<Adapter, String> {
    with_binary(&agent.name, agent.binary.clone())
        .map(|(adapter, _)| adapter)
        .ok_or_else(|| unknown(&agent.name))
}

pub(crate) fn modes(name: &str) -> &'static [PermissionMode] {
    by_name(name).map_or(&[], |adapter| adapter.modes())
}

pub(crate) fn hooked_up(adapter: &Adapter, name: &str, orch_program: &Path) -> Result<(), String> {
    let Some(hookup) = adapter.hookup() else {
        return Ok(());
    };
    let problem = match hookup.state(&orch_program.to_string_lossy(), &xdg::process_env) {
        HookupState::Installed => return Ok(()),
        HookupState::Missing => "is not installed".to_string(),
        HookupState::Broken(problem) => format!("is no longer in place ({problem})"),
    };
    Err(format!(
        "the {name} Agent hookup {problem}; run `orch agent install {name}` first"
    ))
}

pub(crate) fn default_program(name: &str) -> String {
    with_binary(name, None).map_or_else(|| name.into(), |(_, program)| program)
}

pub(crate) fn installed_adapter(agent: &AgentConfig, repo: &Path) -> Result<Adapter, String> {
    let binary = agent
        .binary
        .as_deref()
        .map(|binary| match binary.contains('/') {
            true => repo.join(binary).to_string_lossy().into_owned(),
            false => binary.into(),
        });
    let (adapter, program) =
        with_binary(&agent.name, binary).ok_or_else(|| unknown(&agent.name))?;
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
