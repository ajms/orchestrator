use std::path::Path;

use orch_agent::hook_event::tag_hook_event;
use orch_agent::{AgentAdapter, Antigravity, ConversationTree};
use orch_core::{AgentEvent, ConversationId, GuardedAction, SubagentId};
use serde_json::{Value, json};

const ROOT: &str = "3c1e9a40-7d52-4b8e-a6f1-2d9b0c4e7a13";
const CHILD: &str = "8d2f61b7-4a09-4c3e-9b15-e07a3c5d9f28";

fn fixture_path(name: &str) -> String {
    format!(
        "{}/tests/fixtures/antigravity/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn fixture(name: &str) -> Value {
    let path = fixture_path(&format!("hooks/{name}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path}: {err}"));
    serde_json::from_str(&text).unwrap()
}

fn child(name: &str) -> Value {
    let mut payload = fixture(name);
    payload["transcriptPath"] = fixture_path("transcripts/subagent_full.jsonl").into();
    payload
}

fn tree() -> Box<dyn ConversationTree> {
    let mut tree = Antigravity::default().conversation_tree().unwrap();
    tree.restart(None);
    tree
}

fn hook(tree: &mut Box<dyn ConversationTree>, event: &str, payload: &Value) -> Vec<AgentEvent> {
    tree.hook(&tag_hook_event(&payload.to_string(), event))
        .unwrap()
}

fn tap(tree: &mut Box<dyn ConversationTree>, conversation: &str) -> Vec<AgentEvent> {
    let line = json!({ "conversation_id": conversation, "agent_state": "working" });
    tree.tap(&line.to_string()).unwrap()
}

fn started(agent_type: &str, description: &str) -> AgentEvent {
    AgentEvent::SubagentStarted {
        id: SubagentId(CHILD.into()),
        agent_type: agent_type.into(),
        description: description.into(),
    }
}

fn subagent() -> Option<SubagentId> {
    Some(SubagentId(CHILD.into()))
}

fn finished() -> AgentEvent {
    AgentEvent::SubagentFinished {
        id: SubagentId(CHILD.into()),
    }
}

#[test]
fn another_conversation_is_a_subagent_labelled_by_the_matching_invoke_subagent_spec() {
    let mut tree = tree();
    tap(&mut tree, ROOT);
    assert_eq!(
        hook(
            &mut tree,
            "PreToolUse",
            &fixture("pre_tool_use_invoke_subagent")
        ),
        [AgentEvent::ToolStarted {
            tool: "invoke_subagent".into(),
            subagent: None,
        }]
    );

    assert_eq!(
        hook(
            &mut tree,
            "PreInvocation",
            &child("subagent_pre_invocation")
        ),
        [started("general", "Test Runner")]
    );
}

#[test]
fn an_unmatched_subagent_is_labelled_by_the_first_line_of_its_prompt() {
    let mut tree = tree();
    tap(&mut tree, ROOT);
    assert_eq!(
        hook(
            &mut tree,
            "PreInvocation",
            &child("subagent_pre_invocation")
        )[0],
        started("subagent", "Run the login tests and report the failures.")
    );
}

#[test]
fn a_subagents_tools_are_its_own_and_guarded() {
    let mut tree = tree();
    tap(&mut tree, ROOT);
    hook(
        &mut tree,
        "PreInvocation",
        &child("subagent_pre_invocation"),
    );
    assert_eq!(
        hook(
            &mut tree,
            "PreToolUse",
            &child("subagent_pre_tool_use_run_command")
        ),
        [
            AgentEvent::ToolStarted {
                tool: "run_command".into(),
                subagent: subagent(),
            },
            AgentEvent::GuardCheck {
                tool: "run_command".into(),
                action: GuardedAction::Shell {
                    command: "cargo test login".into(),
                },
                cwd: Some("/home/dev/shop/.orchestrator/worktrees/fix-login".into()),
            },
        ]
    );
    assert_eq!(
        hook(&mut tree, "PostToolUse", &child("subagent_post_tool_use")),
        [AgentEvent::ToolFinished {
            tool: String::new(),
            subagent: subagent(),
        }]
    );
}

#[test]
fn a_subagents_stop_finishes_it_and_a_later_hook_reopens_the_same_row() {
    let mut tree = tree();
    tap(&mut tree, ROOT);
    hook(
        &mut tree,
        "PreToolUse",
        &fixture("pre_tool_use_invoke_subagent"),
    );
    hook(
        &mut tree,
        "PreInvocation",
        &child("subagent_pre_invocation"),
    );
    assert_eq!(
        hook(&mut tree, "Stop", &child("subagent_stop")),
        [AgentEvent::TurnEnded, finished()]
    );

    assert_eq!(
        hook(
            &mut tree,
            "PreInvocation",
            &child("subagent_pre_invocation")
        ),
        [started("general", "Test Runner")]
    );
}

#[test]
fn a_failed_subagent_finishes_without_failing_the_session() {
    let mut tree = tree();
    tap(&mut tree, ROOT);
    hook(
        &mut tree,
        "PreInvocation",
        &child("subagent_pre_invocation"),
    );
    assert_eq!(
        hook(&mut tree, "Stop", &child("subagent_stop_error")),
        [AgentEvent::TurnEnded, finished()]
    );
}

#[test]
fn the_first_hook_after_launch_is_the_sessions_own_conversation() {
    let mut tree = tree();
    assert_eq!(
        hook(&mut tree, "PreInvocation", &fixture("pre_invocation")),
        [
            AgentEvent::ConversationChanged {
                id: ConversationId(ROOT.into())
            },
            AgentEvent::PromptSubmitted
        ]
    );
    assert_eq!(
        hook(&mut tree, "PreInvocation", &fixture("pre_invocation")),
        [AgentEvent::PromptSubmitted]
    );

    tree.restart(None);
    assert_eq!(
        hook(
            &mut tree,
            "PreInvocation",
            &child("subagent_pre_invocation")
        )[0],
        AgentEvent::ConversationChanged {
            id: ConversationId(CHILD.into())
        }
    );
}

#[test]
fn a_new_conversation_on_the_statusline_is_a_conversation_change_not_a_subagent() {
    let mut tree = tree();
    tap(&mut tree, ROOT);
    assert!(
        tap(&mut tree, CHILD).contains(&AgentEvent::ConversationChanged {
            id: ConversationId(CHILD.into())
        })
    );
    assert_eq!(
        hook(
            &mut tree,
            "PreInvocation",
            &child("subagent_pre_invocation")
        ),
        [AgentEvent::PromptSubmitted]
    );
}

#[test]
fn a_nested_subagent_is_flattened_under_the_session() {
    const GRANDCHILD: &str = "5b7e0c94-1f3a-4d62-8a08-c4e9f2b17d35";
    let mut tree = tree();
    tap(&mut tree, ROOT);
    hook(
        &mut tree,
        "PreInvocation",
        &child("subagent_pre_invocation"),
    );
    let mut invoke = fixture("pre_tool_use_invoke_subagent");
    invoke["conversationId"] = CHILD.into();
    hook(&mut tree, "PreToolUse", &invoke);

    let mut grandchild = child("subagent_pre_invocation");
    grandchild["conversationId"] = GRANDCHILD.into();
    assert_eq!(
        hook(&mut tree, "PreInvocation", &grandchild)[0],
        AgentEvent::SubagentStarted {
            id: SubagentId(GRANDCHILD.into()),
            agent_type: "general".into(),
            description: "Test Runner".into(),
        }
    );
}

#[test]
fn after_a_restart_a_known_subagent_is_not_taken_for_the_sessions_conversation() {
    let mut tree = tree();
    tap(&mut tree, ROOT);
    hook(
        &mut tree,
        "PreInvocation",
        &child("subagent_pre_invocation"),
    );
    tree.restart(None);
    assert_eq!(
        hook(
            &mut tree,
            "PreInvocation",
            &child("subagent_pre_invocation")
        ),
        [started(
            "subagent",
            "Run the login tests and report the failures."
        )]
    );
}

fn child_at(name: &str, transcript: &Path) -> Value {
    let mut payload = fixture(name);
    payload["transcriptPath"] = transcript.to_str().unwrap().into();
    payload
}

#[test]
fn a_subagent_whose_transcript_appears_after_its_first_hook_is_relabelled_by_its_spec() {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("transcript_full.jsonl");
    let mut tree = tree();
    tap(&mut tree, ROOT);
    hook(
        &mut tree,
        "PreToolUse",
        &fixture("pre_tool_use_invoke_subagent"),
    );
    assert_eq!(
        hook(
            &mut tree,
            "PreInvocation",
            &child_at("subagent_pre_invocation", &transcript)
        ),
        [started("subagent", "")]
    );

    std::fs::copy(fixture_path("transcripts/subagent_full.jsonl"), &transcript).unwrap();
    assert_eq!(
        hook(
            &mut tree,
            "PostToolUse",
            &child_at("subagent_post_tool_use", &transcript)
        ),
        [
            started("general", "Test Runner"),
            AgentEvent::ToolFinished {
                tool: String::new(),
                subagent: subagent(),
            }
        ]
    );
}

#[test]
fn a_subagent_hook_naming_the_plain_transcript_is_labelled_from_the_full_one() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        fixture_path("transcripts/subagent_full.jsonl"),
        dir.path().join("transcript_full.jsonl"),
    )
    .unwrap();
    let mut tree = tree();
    tap(&mut tree, ROOT);
    hook(
        &mut tree,
        "PreToolUse",
        &fixture("pre_tool_use_invoke_subagent"),
    );
    assert_eq!(
        hook(
            &mut tree,
            "PreInvocation",
            &child_at(
                "subagent_pre_invocation",
                &dir.path().join("transcript.jsonl")
            )
        ),
        [started("general", "Test Runner")]
    );
}
