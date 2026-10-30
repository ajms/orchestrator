use orch_agent::{Capabilities, LaunchSpec, PI, Preset, adapter_named};
use orch_core::{ConversationId, PermissionMode, SessionId};

const SESSION: &str = "5f0c6a52-6a3e-4d3b-9d7e-0b1f8c1e2a90";

fn pi() -> orch_agent::Adapter {
    adapter_named(PI, None).expect("pi is registered")
}

fn spec() -> LaunchSpec {
    LaunchSpec::new(
        SessionId(SESSION.into()),
        "/opt/orch/bin/orch",
        Preset::inherit(),
    )
    .with_worktree("/repos/app/.orchestrator/worktrees/fix")
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| arg.to_string()).collect()
}

#[test]
fn pi_can_only_resume() {
    let resume_only = Capabilities {
        resume: true,
        ..Capabilities::default()
    };
    assert_eq!(pi().capabilities(), resume_only);
}

#[test]
fn launch_opens_a_conversation_named_after_the_session_without_project_config() {
    let argv = pi().launch(&spec());
    assert_eq!(argv.program, "pi");
    assert_eq!(argv.args, strings(&["-na", "--session-id", SESSION]));
}

#[test]
fn launch_passes_the_initial_prompt_after_the_options() {
    let argv = pi().launch(&spec().with_prompt("-fix the bug"));
    assert_eq!(
        argv.args,
        strings(&["-na", "--session-id", SESSION, "--", "-fix the bug"])
    );
}

#[test]
fn the_launch_names_its_conversation_up_front() {
    assert_eq!(
        pi().launch_conversation(&spec()),
        Some(ConversationId(SESSION.into()))
    );
}

#[test]
fn resume_reopens_the_latest_conversation_without_the_prompt() {
    let conversation = ConversationId("conv-7".into());
    let argv = pi()
        .resume(
            &spec().with_prompt("fix the bug"),
            &conversation,
            Some(PermissionMode::Plan),
        )
        .expect("pi resumes");
    assert_eq!(argv.args, strings(&["-na", "--session-id", "conv-7"]));
}

#[test]
fn a_configured_binary_replaces_pi() {
    let pi = adapter_named(PI, Some("/home/dev/.local/bin/pi")).unwrap();
    assert_eq!(pi.launch(&spec()).program, "/home/dev/.local/bin/pi");
}
