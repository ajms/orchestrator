use orch_agent::{Capabilities, LaunchSpec, PI, Preset, adapter_named};
use std::path::PathBuf;

use orch_core::{ConversationId, PermissionMode, SessionId};

const SESSION: &str = "5f0c6a52-6a3e-4d3b-9d7e-0b1f8c1e2a90";
const PROFILE: &str = "/state/orchestrator/agent-profiles/pi";

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
    .with_agent_profile(PROFILE)
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| arg.to_string()).collect()
}

fn orch_extension(args: &[String]) -> Vec<String> {
    let at = args
        .iter()
        .position(|arg| arg == "-e")
        .expect("orch's extension is loaded");
    assert_eq!(
        args[at + 1],
        format!("{PROFILE}/extensions/orch/extension/index.ts")
    );
    args[..at].iter().chain(&args[at + 2..]).cloned().collect()
}

#[test]
fn pi_reports_hooks_titles_and_usage_and_resumes() {
    assert_eq!(
        pi().capabilities(),
        Capabilities {
            hooks: true,
            resume: true,
            titles: true,
            usage: true,
            ..Capabilities::default()
        }
    );
}

#[test]
fn launch_opens_a_conversation_named_after_the_session_without_project_config() {
    let argv = pi().launch(&spec());
    assert_eq!(argv.program, "pi");
    assert_eq!(
        orch_extension(&argv.args),
        strings(&[
            "-na",
            "--session-id",
            SESSION,
            "--orch-bin",
            "/opt/orch/bin/orch",
            "--orch-session",
            SESSION,
        ])
    );
}

#[test]
fn launch_passes_the_initial_prompt_after_the_options() {
    let argv = pi().launch(&spec().with_prompt("-fix the bug"));
    assert_eq!(
        argv.args[argv.args.len() - 2..],
        strings(&["--", "-fix the bug"])
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
    assert_eq!(
        orch_extension(&argv.args),
        strings(&[
            "-na",
            "--session-id",
            "conv-7",
            "--orch-bin",
            "/opt/orch/bin/orch",
            "--orch-session",
            SESSION,
        ])
    );
}

#[test]
fn a_configured_binary_replaces_pi() {
    let pi = adapter_named(PI, Some("/home/dev/.local/bin/pi")).unwrap();
    assert_eq!(pi.launch(&spec()).program, "/home/dev/.local/bin/pi");
}

fn lookup(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
    move |key| {
        vars.iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.to_string())
    }
}

#[test]
fn the_agent_profile_lives_in_orchs_state_dir() {
    assert_eq!(
        pi().agent_profile(&lookup(&[("HOME", "/home/dev")])),
        Ok(Some(PathBuf::from(
            "/home/dev/.local/state/orchestrator/agent-profiles/pi"
        )))
    );
    assert_eq!(
        pi().agent_profile(&lookup(&[
            ("HOME", "/home/dev"),
            ("XDG_STATE_HOME", "/state")
        ])),
        Ok(Some(PathBuf::from(PROFILE)))
    );
}

#[test]
fn without_home_there_is_no_agent_profile_to_launch_from() {
    assert!(pi().agent_profile(&lookup(&[])).is_err());
}
