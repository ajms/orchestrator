mod common;

use common::*;
use orch_core::{AgentEvent, ConversationId, PermissionMode, Subagent, SubagentId, UsageSample};

fn subagent_started(id: &str) -> AgentEvent {
    AgentEvent::SubagentStarted {
        id: SubagentId(id.into()),
        agent_type: "Explore".into(),
        description: "map the code".into(),
    }
}

fn tool_in(id: &str) -> AgentEvent {
    AgentEvent::ToolStarted {
        tool: "Grep".into(),
        subagent: Some(SubagentId(id.into())),
    }
}

#[test]
fn the_last_observed_mode_and_conversation_are_remembered() {
    let mut status = idle_session();
    assert_eq!(status.permission_mode(), None);
    assert_eq!(status.conversation(), None);

    status.feed_event(AgentEvent::ModeChanged {
        mode: PermissionMode::AcceptEdits,
    });
    status.feed_event(AgentEvent::ConversationChanged {
        id: ConversationId("c1".into()),
    });
    status.feed_event(AgentEvent::ConversationChanged {
        id: ConversationId("c2".into()),
    });

    assert_eq!(status.permission_mode(), Some(PermissionMode::AcceptEdits));
    assert_eq!(status.conversation(), Some(&ConversationId("c2".into())));
}

#[test]
fn the_latest_usage_sample_is_kept() {
    let mut status = idle_session();
    let sample = UsageSample {
        context_used_percent: Some(42.0),
        ..Default::default()
    };
    status.feed_event(AgentEvent::UsageSample(sample.clone()));
    assert_eq!(status.usage(), Some(&sample));
}

#[test]
fn subagents_are_listed_with_their_tool_count_until_done() {
    let mut status = working_session();
    status.feed_event(subagent_started("a1"));
    status.feed_event(subagent_started("a2"));
    status.feed_event(tool_in("a1"));
    status.feed_event(tool_in("a1"));
    status.feed_event(AgentEvent::SubagentFinished {
        id: SubagentId("a2".into()),
    });

    assert_eq!(
        status.subagents(),
        [
            Subagent {
                id: SubagentId("a1".into()),
                agent_type: "Explore".into(),
                description: "map the code".into(),
                tool_count: 2,
                done: false,
            },
            Subagent {
                id: SubagentId("a2".into()),
                agent_type: "Explore".into(),
                description: "map the code".into(),
                tool_count: 0,
                done: true,
            },
        ]
    );
}
