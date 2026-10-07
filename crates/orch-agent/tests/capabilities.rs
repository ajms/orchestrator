use orch_agent::{
    AgentAdapter, Antigravity, Argv, Capabilities, ClaudeCode, LaunchSpec, Preset, Presets,
};
use orch_core::{AgentState, ConversationId, Observation, PermissionMode, PhaseEvent, SessionId};
use std::time::Instant;

struct Bare;

impl AgentAdapter for Bare {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn launch(&self, spec: &LaunchSpec) -> Argv {
        Argv {
            program: "bare".into(),
            args: vec![spec.session.as_str().into()],
        }
    }
}

fn spec(preset: Preset) -> LaunchSpec {
    LaunchSpec::new(SessionId("s-1".into()), "orch", preset)
}

fn conversation() -> ConversationId {
    ConversationId("c-1".into())
}

fn edits() -> Preset {
    Presets::default().get("edits").unwrap().clone()
}

#[test]
fn claude_declares_every_capability() {
    assert_eq!(
        ClaudeCode::default().capabilities(),
        Capabilities {
            hooks: true,
            resume: true,
            usage: true,
            modes: true,
            guards: true,
            subagents: true,
            titles: true,
            transcripts: true,
        }
    );
}

#[test]
fn an_agent_without_hooks_is_unobserved_and_its_state_unknown() {
    let mut status = Bare.capabilities().session_status();
    status.transition(PhaseEvent::SetupSucceeded).unwrap();
    status.observe(Observation::Spawned, Instant::now());
    assert_eq!(status.agent_state(), Some(AgentState::Unknown));

    let mut status = ClaudeCode::default().capabilities().session_status();
    status.transition(PhaseEvent::SetupSucceeded).unwrap();
    status.observe(Observation::Spawned, Instant::now());
    assert_eq!(status.agent_state(), Some(AgentState::Starting));
}

#[test]
fn guards_need_both_the_guards_capability_and_hooks() {
    assert!(ClaudeCode::default().capabilities().guards_available());
    assert!(!Bare.capabilities().guards_available());
    let guards_without_hooks = Capabilities {
        guards: true,
        ..Capabilities::default()
    };
    assert!(!guards_without_hooks.guards_available());
}

#[test]
fn an_agent_without_resume_restarts_in_a_fresh_conversation() {
    let restart = Bare.restart(&spec(Preset::inherit()), Some(&conversation()), None);
    assert_eq!(restart, Bare.launch(&spec(Preset::inherit())));
    assert_eq!(Bare.draft(&conversation()), None);
}

#[test]
fn claude_restarts_by_resuming_the_latest_conversation() {
    let claude = ClaudeCode::default();
    let mode = Some(PermissionMode::Plan);
    assert_eq!(
        claude.restart(&spec(edits()), Some(&conversation()), mode),
        claude
            .resume(&spec(edits()), &conversation(), mode)
            .unwrap()
    );
    assert_eq!(
        claude.restart(&spec(edits()), None, mode),
        claude.launch(&spec(edits()))
    );
}

#[test]
fn an_antigravity_session_is_observed_and_runs_its_preset() {
    let capabilities = Antigravity::default().capabilities();
    let mut status = capabilities.session_status();
    status.transition(PhaseEvent::SetupSucceeded).unwrap();
    status.observe(Observation::Spawned, Instant::now());
    assert_eq!(status.agent_state(), Some(AgentState::Starting));
    assert_eq!(capabilities.effective_preset(edits()), edits());
}

#[test]
fn an_agent_without_modes_only_runs_inherit() {
    assert_eq!(
        Bare.capabilities().effective_preset(edits()),
        Preset::inherit()
    );
    assert_eq!(
        ClaudeCode::default()
            .capabilities()
            .effective_preset(edits()),
        edits()
    );
}

#[test]
fn an_agent_without_hooks_or_usage_maps_nothing() {
    assert_eq!(Bare.map_hook(r#"{"hook_event_name":"Stop"}"#), Ok(vec![]));
    assert_eq!(Bare.map_tap("{}"), Ok(vec![]));
    assert!(Bare.title_watch().is_none());
    assert!(Bare.subagent_transcripts().is_none());
}
