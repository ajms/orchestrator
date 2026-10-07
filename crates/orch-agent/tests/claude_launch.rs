use orch_agent::{AgentAdapter, ClaudeCode, LaunchSpec, Preset, Presets};
use orch_core::{ConversationId, PermissionMode, SessionId};
use serde_json::Value;

const SESSION: &str = "5f0c6a52-6a3e-4d3b-9d7e-0b1f8c1e2a90";

fn spec(preset: Preset) -> LaunchSpec {
    LaunchSpec::new(SessionId(SESSION.into()), "/opt/orch/bin/orch", preset)
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == name)
        .map(|at| args[at + 1].as_str())
}

fn settings(args: &[String]) -> Value {
    serde_json::from_str(flag(args, "--settings").expect("--settings passed")).unwrap()
}

#[test]
fn launch_pins_our_session_id_as_the_first_conversation() {
    let argv = ClaudeCode::default().launch(&spec(Preset::inherit()));
    assert_eq!(argv.program, "claude");
    assert_eq!(flag(&argv.args, "--session-id"), Some(SESSION));
}

fn hook_commands(settings: &Value, event: &str) -> Vec<String> {
    settings["hooks"][event]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|group| group["hooks"].as_array().cloned().unwrap_or_default())
        .map(|hook| hook["command"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn launch_routes_every_observed_hook_to_the_orch_hook_shim() {
    let argv = ClaudeCode::default().launch(&spec(Preset::inherit()));
    let settings = settings(&argv.args);
    let shim = format!("'/opt/orch/bin/orch' hook --session '{SESSION}'");
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PostToolUseFailure",
        "PermissionRequest",
        "PermissionDenied",
        "Notification",
        "Stop",
        "StopFailure",
        "SubagentStart",
        "SubagentStop",
    ] {
        assert_eq!(
            hook_commands(&settings, event),
            vec![shim.clone()],
            "{event}"
        );
    }
}

#[test]
fn guard_hook_waits_for_the_users_answer_instead_of_timing_out() {
    let argv = ClaudeCode::default().launch(&spec(Preset::inherit()));
    let settings = settings(&argv.args);
    let timeout = settings["hooks"]["PreToolUse"][0]["hooks"][0]["timeout"]
        .as_u64()
        .unwrap();
    assert!(timeout >= 24 * 60 * 60, "timeout {timeout}s");
}

#[test]
fn launch_taps_the_statusline_with_the_session_id_in_its_command() {
    let argv = ClaudeCode::default().launch(&spec(Preset::inherit()));
    let status_line = &settings(&argv.args)["statusLine"];
    assert_eq!(status_line["type"], "command");
    assert_eq!(
        status_line["command"],
        format!("'/opt/orch/bin/orch' tap --session '{SESSION}'")
    );
}

#[test]
fn orch_program_paths_are_shell_quoted_in_injected_commands() {
    let spec = LaunchSpec::new(
        SessionId(SESSION.into()),
        "/home/me/it's here/orch",
        Preset::inherit(),
    );
    let argv = ClaudeCode::default().launch(&spec);
    assert_eq!(
        settings(&argv.args)["statusLine"]["command"],
        format!("'/home/me/it'\\''s here/orch' tap --session '{SESSION}'")
    );
}

#[test]
fn launch_passes_the_presets_permission_mode() {
    let presets = Presets::default();
    for (name, mode) in [
        ("plan", "plan"),
        ("ask", "default"),
        ("edits", "acceptEdits"),
        ("auto", "auto"),
    ] {
        let argv = ClaudeCode::default().launch(&spec(presets.get(name).unwrap().clone()));
        assert_eq!(flag(&argv.args, "--permission-mode"), Some(mode), "{name}");
    }
}

#[test]
fn inherit_leaves_the_users_own_default_mode_alone() {
    let argv = ClaudeCode::default().launch(&spec(Preset::inherit()));
    assert_eq!(flag(&argv.args, "--permission-mode"), None);
    assert!(settings(&argv.args).get("permissions").is_none());
}

#[test]
fn launch_adds_the_presets_rules_to_the_users_permissions() {
    let preset = Preset {
        name: "locked".into(),
        mode: Some(PermissionMode::DontAsk),
        allow: vec!["Bash(cargo test *)".into()],
        deny: vec!["WebFetch".into(), "Bash(rm *)".into()],
    };
    let argv = ClaudeCode::default().launch(&spec(preset));
    assert_eq!(flag(&argv.args, "--permission-mode"), Some("dontAsk"));
    assert_eq!(
        settings(&argv.args)["permissions"],
        serde_json::json!({
            "allow": ["Bash(cargo test *)"],
            "deny": ["WebFetch", "Bash(rm *)"],
        })
    );
}

fn conversation() -> ConversationId {
    ConversationId("9a1d7f3e-2b4c-4e8a-b1d2-6c3f5e7a9b0c".into())
}

#[test]
fn resume_continues_the_latest_conversation_with_our_settings_again() {
    let launch = ClaudeCode::default().launch(&spec(Preset::inherit()));
    let resume = ClaudeCode::default()
        .resume(&spec(Preset::inherit()), &conversation(), None)
        .expect("claude can resume");
    assert_eq!(resume.program, "claude");
    assert_eq!(
        flag(&resume.args, "--resume"),
        Some(conversation().as_str())
    );
    assert_eq!(flag(&resume.args, "--session-id"), None);
    assert_eq!(settings(&resume.args), settings(&launch.args));
    assert_eq!(flag(&resume.args, "--permission-mode"), None);
}

#[test]
fn resume_comes_back_in_the_given_mode_rather_than_the_presets() {
    let edits = Presets::default().get("edits").unwrap().clone();
    let resume = ClaudeCode::default()
        .resume(&spec(edits), &conversation(), Some(PermissionMode::Plan))
        .unwrap();
    assert_eq!(flag(&resume.args, "--permission-mode"), Some("plan"));
}

#[test]
fn draft_forks_the_conversation_in_print_mode_without_our_hooks() {
    let draft = ClaudeCode::default()
        .draft(&conversation())
        .expect("claude can draft");
    assert_eq!(draft.program, "claude");
    assert!(draft.args.contains(&"-p".to_string()));
    assert!(draft.args.contains(&"--fork-session".to_string()));
    assert_eq!(flag(&draft.args, "--resume"), Some(conversation().as_str()));
    assert_eq!(flag(&draft.args, "--settings"), None);
}

#[test]
fn resume_falls_back_to_the_presets_mode_when_no_mode_was_observed() {
    let edits = Presets::default().get("edits").unwrap().clone();
    let resume = ClaudeCode::default()
        .resume(&spec(edits), &conversation(), None)
        .unwrap();
    assert_eq!(flag(&resume.args, "--permission-mode"), Some("acceptEdits"));
}

#[test]
fn resume_under_inherit_leaves_the_mode_to_claudes_own_restore() {
    let resume = ClaudeCode::default()
        .resume(
            &spec(Preset::inherit()),
            &conversation(),
            Some(PermissionMode::Plan),
        )
        .unwrap();
    assert_eq!(flag(&resume.args, "--permission-mode"), None);
}

#[test]
fn launch_passes_the_initial_prompt_as_the_last_argument() {
    let spec = spec(Preset::inherit()).with_prompt("fix the --flaky test");
    let argv = ClaudeCode::default().launch(&spec);
    assert_eq!(
        argv.args.last().map(String::as_str),
        Some("fix the --flaky test")
    );
    assert_eq!(argv.args[argv.args.len() - 2], "--");
}

#[test]
fn resume_never_repeats_the_initial_prompt() {
    let spec = spec(Preset::inherit()).with_prompt("fix the flaky test");
    let conversation = ConversationId("c0ffee".into());
    let argv = ClaudeCode::default()
        .resume(&spec, &conversation, None)
        .unwrap();
    assert!(!argv.args.iter().any(|arg| arg.contains("flaky")));
}

#[test]
fn launch_routes_subagent_worktrees_into_the_sessions_worktree_hook() {
    let spec = spec(Preset::inherit()).with_worktree("/repo/.orchestrator/worktrees/work");
    let settings = settings(&ClaudeCode::default().launch(&spec).args);
    let shim = "'/opt/orch/bin/orch' worktree-hook --worktree '/repo/.orchestrator/worktrees/work'";
    for event in ["WorktreeCreate", "WorktreeRemove"] {
        assert_eq!(hook_commands(&settings, event), vec![shim], "{event}");
    }
}

#[test]
fn launch_without_a_worktree_leaves_subagent_worktrees_to_claude() {
    let settings = settings(&ClaudeCode::default().launch(&spec(Preset::inherit())).args);
    assert!(hook_commands(&settings, "WorktreeCreate").is_empty());
    assert!(hook_commands(&settings, "WorktreeRemove").is_empty());
}
