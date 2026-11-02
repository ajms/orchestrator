use orch_agent::{CLAUDE, LaunchSpec, Preset, adapter_named};
use orch_core::SessionId;

fn spec() -> LaunchSpec {
    LaunchSpec::new(SessionId("s-1".into()), "orch", Preset::inherit())
}

#[test]
fn claude_is_known_by_its_config_name() {
    let claude = adapter_named(CLAUDE, None).expect("claude is registered");
    assert_eq!(claude.launch(&spec()).program, "claude");
}

#[test]
fn a_configured_binary_replaces_the_agents_default_program() {
    let claude = adapter_named(CLAUDE, Some("/opt/claude/bin/claude")).unwrap();
    assert_eq!(claude.launch(&spec()).program, "/opt/claude/bin/claude");
}

#[test]
fn an_unknown_agent_has_no_adapter() {
    assert!(adapter_named("nope", None).is_none());
}
