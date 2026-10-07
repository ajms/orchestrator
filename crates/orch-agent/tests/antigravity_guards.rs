use std::path::{Path, PathBuf};

use orch_agent::{
    AgentAdapter, Antigravity, GuardAnswer, GuardContext, GuardDecision, GuardHit, GuardKind,
    GuardOutcome, RuleVerdict, evaluate_guard, guard_outcome, tag_hook_event,
};
use orch_core::{AgentEvent, GuardedAction};
use serde_json::{Value, json};

const WORKTREE: &str = "/home/dev/shop/.orchestrator/worktrees/fix-login";

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/antigravity/hooks/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path}: {err}"))
}

fn guard_check(payload: &str) -> Option<(String, GuardedAction, Option<String>)> {
    Antigravity::default()
        .map_hook(&tag_hook_event(payload, "PreToolUse"))
        .unwrap()
        .into_iter()
        .find_map(|event| match event {
            AgentEvent::GuardCheck { tool, action, cwd } => Some((tool, action, cwd)),
            _ => None,
        })
}

fn renamed(name: &str, tool: &str) -> String {
    let mut payload: Value = serde_json::from_str(&fixture(name)).unwrap();
    payload["toolCall"]["name"] = json!(tool);
    payload.to_string()
}

#[test]
fn every_file_writing_tool_writes_its_target_file() {
    let path = format!("{WORKTREE}/src/login.ts");
    for tool in [
        "write_to_file",
        "write_file",
        "create_file",
        "edit_file",
        "replace_file_content",
        "multi_replace_file_content",
        "delete_file",
        "edit_notebook",
    ] {
        let (shown, action, _) = guard_check(&renamed("pre_tool_use_write_to_file", tool))
            .unwrap_or_else(|| panic!("{tool} is not guarded"));
        assert_eq!(shown, tool);
        assert_eq!(
            action,
            GuardedAction::WriteFile { path: path.clone() },
            "{tool}"
        );
    }
}

#[test]
fn a_command_runs_in_its_own_directory() {
    assert_eq!(
        guard_check(&fixture("pre_tool_use_run_command")),
        Some((
            "run_command".into(),
            GuardedAction::Shell {
                command: "npm test".into()
            },
            Some(WORKTREE.into()),
        ))
    );
}

#[test]
fn input_sent_to_a_running_command_is_a_shell_command() {
    assert_eq!(
        guard_check(&fixture("pre_tool_use_send_command_input")),
        Some((
            "send_command_input".into(),
            GuardedAction::Shell {
                command: "git checkout main\n".into()
            },
            None,
        ))
    );
}

#[test]
fn mcp_and_browser_tools_reach_beyond_the_session() {
    for (name, tool) in [
        (
            "pre_tool_use_mcp_tool",
            "mcp_chrome_devtools_take_memory_snapshot",
        ),
        ("pre_tool_use_open_browser_url", "open_browser_url"),
    ] {
        let (_, action, _) = guard_check(&fixture(name)).unwrap();
        assert_eq!(action, GuardedAction::ExternalTool { name: tool.into() });
    }
    let (_, action, _) = guard_check(&renamed(
        "pre_tool_use_open_browser_url",
        "browser_subagent",
    ))
    .unwrap();
    assert_eq!(
        action,
        GuardedAction::ExternalTool {
            name: "browser_subagent".into()
        }
    );
}

#[test]
fn reading_and_asking_are_not_guarded() {
    assert_eq!(
        guard_check(&renamed("pre_tool_use_write_to_file", "view_file")),
        None
    );
    assert_eq!(guard_check(&fixture("pre_tool_use_ask_question")), None);
}

fn without_arg(name: &str, arg: &str) -> String {
    let mut payload: Value = serde_json::from_str(&fixture(name)).unwrap();
    payload["toolCall"]["args"]
        .as_object_mut()
        .unwrap()
        .remove(arg);
    payload.to_string()
}

#[test]
fn a_guarded_tool_whose_target_orch_cannot_read_is_still_checked() {
    for (name, arg) in [
        ("pre_tool_use_write_to_file", "TargetFile"),
        ("pre_tool_use_run_command", "CommandLine"),
        ("pre_tool_use_send_command_input", "Input"),
    ] {
        let (_, action, _) = guard_check(&without_arg(name, arg))
            .unwrap_or_else(|| panic!("{name} without {arg} is not guarded"));
        assert_eq!(action, GuardedAction::Unreadable, "{name}");
    }
}

#[test]
fn an_unreadable_tool_call_makes_agy_ask_the_user_even_with_guards_off() {
    for enabled in [true, false] {
        let context = GuardContext {
            worktree: Path::new(WORKTREE),
            branch: "orch/fix-login",
            base_branch: "main",
            enabled,
            allowed: &[],
            agent_dirs: &[],
        };
        let decision = evaluate_guard(&GuardedAction::Unreadable, None, &context);
        assert_eq!(
            answered(guard_outcome(decision, None)),
            json!({ "decision": "force_ask" })
        );
    }
}

fn reply(answer: &GuardAnswer) -> Value {
    serde_json::from_str(&Antigravity::default().guard_answer(answer).unwrap()).unwrap()
}

#[test]
fn every_guard_answer_carries_an_explicit_decision() {
    assert_eq!(reply(&GuardAnswer::Proceed), json!({ "decision": "ask" }));
    assert_eq!(reply(&GuardAnswer::Ask), json!({ "decision": "force_ask" }));
    assert_eq!(
        reply(&GuardAnswer::PresetAllow),
        json!({ "decision": "allow" })
    );
    assert_eq!(
        reply(&GuardAnswer::Deny {
            reason: "touches main".into()
        }),
        json!({ "decision": "deny", "reason": "touches main" })
    );
}

fn hit() -> GuardHit {
    GuardHit {
        kind: GuardKind::BaseBranch,
        target: "main".into(),
    }
}

fn allow_rule() -> Option<RuleVerdict> {
    Some(RuleVerdict::Allow {
        rule: "command(git)".into(),
    })
}

fn deny_rule() -> Option<RuleVerdict> {
    Some(RuleVerdict::Deny {
        rule: "command(git)".into(),
    })
}

fn answered(outcome: GuardOutcome) -> Value {
    match outcome {
        GuardOutcome::Answer(answer) => reply(&answer),
        GuardOutcome::Prompt { .. } => panic!("expected an answer, got {outcome:?}"),
    }
}

#[test]
fn without_a_guard_hit_preset_rules_decide_and_otherwise_agy_asks() {
    let allow = GuardDecision::Allow;
    assert_eq!(
        answered(guard_outcome(allow.clone(), None)),
        json!({ "decision": "ask" })
    );
    assert_eq!(
        answered(guard_outcome(allow.clone(), allow_rule())),
        json!({ "decision": "allow" })
    );
    assert_eq!(
        answered(guard_outcome(allow, deny_rule()))["decision"],
        "deny"
    );
}

#[test]
fn a_preset_deny_names_its_rule() {
    let denied = answered(guard_outcome(GuardDecision::Allow, deny_rule()));
    assert!(
        denied["reason"].as_str().unwrap().contains("command(git)"),
        "{denied}"
    );
}

#[test]
fn a_guard_hit_asks_the_user_before_any_preset_allow() {
    for (verdict, allowed) in [
        (None, json!({ "decision": "ask" })),
        (allow_rule(), json!({ "decision": "allow" })),
    ] {
        match guard_outcome(GuardDecision::Ask(hit()), verdict) {
            GuardOutcome::Prompt {
                hit: prompted,
                on_allow: answer,
            } => {
                assert_eq!(prompted, hit());
                assert_eq!(reply(&answer), allowed);
            }
            other => panic!("expected a Guard prompt, got {other:?}"),
        }
    }
}

#[test]
fn a_guard_hit_on_a_preset_denied_call_is_denied_without_asking() {
    assert_eq!(
        answered(guard_outcome(GuardDecision::Ask(hit()), deny_rule()))["decision"],
        "deny"
    );
}

#[test]
fn antigravity_sessions_are_guarded() {
    assert!(Antigravity::default().capabilities().guards_available());
}

#[test]
fn agys_own_dir_is_its_brain() {
    let lookup = |key: &str| (key == "HOME").then(|| "/home/dev".to_string());
    assert_eq!(
        Antigravity::default().agent_dirs(&lookup),
        [PathBuf::from("/home/dev/.gemini/antigravity-cli/brain")]
    );
}
