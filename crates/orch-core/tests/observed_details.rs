mod common;

use common::*;
use orch_core::{
    AgentEvent, ConversationId, PermissionMode, SubagentId, SubagentOutcome, SubagentState,
    UsageSample,
};

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

fn rows(status: &orch_core::SessionStatus) -> Vec<(&str, u32, SubagentState)> {
    status
        .subagents()
        .iter()
        .map(|subagent| (subagent.id.as_str(), subagent.tool_count, subagent.state))
        .collect()
}

fn finished(id: &str, outcome: SubagentOutcome) -> AgentEvent {
    AgentEvent::SubagentFinished {
        id: SubagentId(id.into()),
        outcome,
    }
}

#[test]
fn subagents_are_listed_with_their_tool_count_until_done() {
    let mut status = working_session();
    status.feed_event(subagent_started("a1"));
    status.feed_event(subagent_started("a2"));
    status.feed_event(tool_in("a1"));
    status.feed_event(tool_in("a1"));
    status.feed_event(finished("a2", SubagentOutcome::default()));

    assert_eq!(
        rows(&status),
        [
            ("a1", 2, SubagentState::Working),
            ("a2", 0, SubagentState::Done)
        ]
    );
    let a1 = &status.subagents()[0];
    assert_eq!(
        (a1.agent_type.as_str(), a1.description.as_str()),
        ("Explore", "map the code")
    );
    assert!(a1.started_at.is_some());
    assert_eq!(a1.duration, None);
    assert!(status.subagents()[1].duration.is_some());
}

#[test]
fn a_subagent_that_reports_an_error_is_errored_with_its_outcome() {
    let mut status = working_session();
    status.feed_event(subagent_started("a1"));
    let outcome = SubagentOutcome {
        errored: true,
        failure: Some("merge conflict".into()),
        ..SubagentOutcome::default()
    };
    status.feed_event(finished("a1", outcome.clone()));

    assert_eq!(rows(&status), [("a1", 0, SubagentState::Errored)]);
    assert_eq!(status.subagents()[0].outcome, outcome);
    assert!(status.subagents()[0].finished());
}

#[test]
fn a_resumed_subagent_runs_again_in_its_own_row() {
    let mut status = working_session();
    status.feed_event(subagent_started("a1"));
    status.feed_event(tool_in("a1"));
    status.feed_event(finished(
        "a1",
        SubagentOutcome {
            errored: true,
            ..SubagentOutcome::default()
        },
    ));
    status.feed_event(subagent_started("a1"));
    status.feed_event(tool_in("a1"));

    assert_eq!(rows(&status), [("a1", 2, SubagentState::Working)]);
    assert_eq!(status.subagents()[0].outcome, SubagentOutcome::default());
    assert_eq!(status.subagents()[0].duration, None);
}
