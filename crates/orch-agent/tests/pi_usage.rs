use orch_agent::{AgentAdapter, Pi};
use orch_core::{AgentEvent, ConversationId, UsageSample};

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/pi/taps/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path}: {err}"))
}

fn sample(payload: &str) -> UsageSample {
    match Pi::default().map_tap(payload).unwrap().as_slice() {
        [AgentEvent::UsageSample(sample)] => sample.clone(),
        other => panic!("one usage sample expected, got {other:?}"),
    }
}

#[test]
fn a_settled_turn_yields_the_context_gauge_the_last_responses_tokens_and_the_cost() {
    assert_eq!(
        sample(&fixture("agent_settled")),
        UsageSample {
            conversation: Some(ConversationId(
                "d50a54e3-f3a9-4790-b89a-7904f7c63635".into()
            )),
            model: Some("Scripted".into()),
            context_used_percent: Some(3.15),
            context_window_tokens: Some(100_000),
            input_tokens: Some(3_148),
            output_tokens: Some(2),
            cost_usd: Some(0.0106545),
            five_hour: None,
            seven_day: None,
        }
    );
}

#[test]
fn input_tokens_are_the_last_responses_input_with_its_cache_reads_and_writes() {
    let second = sample(&fixture("agent_settled_second_turn"));
    assert_eq!(second.input_tokens, Some(16 + 1_581 + 16));
    assert_eq!(second.output_tokens, Some(2));
    assert_eq!(second.cost_usd, Some(0.01187625));
}

#[test]
fn a_fresh_conversation_reports_zero_tokens_and_cost() {
    let fresh = sample(&fixture("session_start_startup"));
    assert_eq!(
        (fresh.input_tokens, fresh.output_tokens, fresh.cost_usd),
        (Some(0), Some(0), Some(0.0))
    );
    assert_eq!(fresh.context_used_percent, Some(0.0));
}

#[test]
fn a_resume_reports_the_conversation_it_continues() {
    let resumed = sample(&fixture("session_start_resumed"));
    assert_eq!(
        resumed.conversation,
        Some(ConversationId(
            "d50a54e3-f3a9-4790-b89a-7904f7c63635".into()
        ))
    );
    assert_eq!(resumed.input_tokens, Some(6 + 1_747 + 6));
    assert_eq!(resumed.cost_usd, Some(0.03968504999999999));
}

#[test]
fn a_fork_starts_its_cost_at_zero_with_the_context_it_copied() {
    let fork = sample(&fixture("session_start_fork"));
    assert_eq!(
        fork.conversation,
        Some(ConversationId(
            "01a18bf1-3e5c-724e-9417-0e5464e9ea02".into()
        ))
    );
    assert_eq!(fork.cost_usd, Some(0.0));
    assert_eq!(fork.input_tokens, Some(1_574 + 1_574));
    assert_eq!(
        sample(&fixture("agent_settled_fork")).cost_usd,
        Some(0.0107085)
    );
}

#[test]
fn context_unknown_after_a_compaction_leaves_the_gauge_empty() {
    let compacted = sample(
        r#"{"sessionId":"c1","model":{"id":"local","name":""},"context":{"tokens":null,"contextWindow":32768,"percent":null},"lastResponse":{"input":5,"output":1,"cacheRead":0,"cacheWrite":0},"cost":0}"#,
    );
    assert_eq!(compacted.context_used_percent, None);
    assert_eq!(compacted.context_window_tokens, Some(32_768));
    assert_eq!(compacted.model.as_deref(), Some("local"));
    assert_eq!(compacted.cost_usd, Some(0.0));
}

#[test]
fn a_tap_without_model_or_context_still_reports_tokens_and_cost() {
    let bare = sample(
        r#"{"sessionId":"c1","lastResponse":{"input":5,"output":1,"cacheRead":2,"cacheWrite":3},"cost":0.5}"#,
    );
    assert_eq!(bare.model, None);
    assert_eq!(bare.context_used_percent, None);
    assert_eq!(bare.context_window_tokens, None);
    assert_eq!(bare.input_tokens, Some(10));
    assert_eq!(bare.cost_usd, Some(0.5));
}

#[test]
fn garbage_tap_input_is_an_error() {
    assert!(Pi::default().map_tap("{").is_err());
}
