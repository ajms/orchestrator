use orch_agent::{AgentAdapter, Antigravity, LaunchSpec, Preset, Presets};
use orch_core::{ConversationId, PermissionMode, SessionId};

const CONVERSATION: &str = "3c1e9a40-7d52-4b8e-a6f1-2d9b0c4e7a13";

fn preset(name: &str) -> Preset {
    Presets::default().get(name).unwrap().clone()
}

fn spec(preset: Preset) -> LaunchSpec {
    LaunchSpec::new(SessionId("fix-login".into()), "/opt/orch/bin/orch", preset)
}

fn launched(preset: Preset) -> Vec<String> {
    Antigravity::default()
        .launch(&spec(preset).with_prompt("Fix the login bug"))
        .args
}

fn resumed(preset: Preset, observed: Option<PermissionMode>) -> Vec<String> {
    Antigravity::default()
        .resume(
            &spec(preset),
            &ConversationId(CONVERSATION.into()),
            observed,
        )
        .expect("agy resumes")
        .args
}

#[test]
fn launch_runs_agy_with_the_prompt_submitted_interactively() {
    let argv = Antigravity::default().launch(&spec(preset("ask")).with_prompt("Fix the login bug"));
    assert_eq!(argv.program, "agy");
    assert_eq!(argv.args, ["-i", "Fix the login bug"]);
}

#[test]
fn launch_without_a_prompt_just_opens_agy() {
    let argv = Antigravity::default().launch(&spec(preset("ask")));
    assert!(argv.args.is_empty(), "{:?}", argv.args);
}

#[test]
fn launch_sets_agys_mode_from_the_preset() {
    assert_eq!(
        launched(preset("edits")),
        ["--mode", "accept-edits", "-i", "Fix the login bug"]
    );
    assert_eq!(
        launched(preset("plan")),
        ["--mode", "plan", "-i", "Fix the login bug"]
    );
    assert_eq!(launched(Preset::inherit()), ["-i", "Fix the login bug"]);
}

#[test]
fn resume_continues_the_conversation_in_the_last_observed_mode() {
    assert_eq!(
        resumed(preset("edits"), Some(PermissionMode::Plan)),
        ["--conversation", CONVERSATION, "--mode", "plan"]
    );
    assert_eq!(
        resumed(preset("edits"), Some(PermissionMode::Default)),
        ["--conversation", CONVERSATION]
    );
}

#[test]
fn resume_falls_back_to_the_presets_mode_before_any_was_observed() {
    assert_eq!(
        resumed(preset("edits"), None),
        ["--conversation", CONVERSATION, "--mode", "accept-edits"]
    );
}

#[test]
fn an_inherit_session_resumes_in_whatever_mode_agy_defaults_to() {
    assert_eq!(
        resumed(Preset::inherit(), Some(PermissionMode::Plan)),
        ["--conversation", CONVERSATION]
    );
}

#[test]
fn antigravity_offers_only_the_modes_agy_can_launch() {
    let presets = Presets::default();
    let offered: Vec<&str> = presets
        .offered(Antigravity::default().modes())
        .map(|preset| preset.name.as_str())
        .collect();
    assert_eq!(offered, ["plan", "ask", "edits", "inherit"]);
}
