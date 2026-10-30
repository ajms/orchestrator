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
fn a_settled_turn_yields_the_context_gauge_and_the_conversations_totals() {
    assert_eq!(
        sample(&fixture("agent_settled")),
        UsageSample {
            conversation: Some(ConversationId(
                "1476af5a-45a3-495c-afc0-cced3b8aaa0e".into()
            )),
            model: Some("Scripted".into()),
            context_used_percent: Some(3.2),
            context_window_tokens: Some(100_000),
            input_tokens: Some(3_198),
            output_tokens: Some(2),
            cost_usd: Some(0.01082325),
            five_hour: None,
            seven_day: None,
        }
    );
}

#[test]
fn input_tokens_count_cache_reads_and_writes() {
    let second = sample(&fixture("agent_settled_second_turn"));
    assert_eq!(second.input_tokens, Some(1_622 + 3_205 + 1_622));
    assert_eq!(second.output_tokens, Some(10));
    assert_eq!(second.cost_usd, Some(0.01206));
}

#[test]
fn a_resume_reports_the_conversation_it_continues() {
    let resumed = sample(&fixture("session_start_resumed"));
    assert_eq!(
        resumed.conversation,
        Some(ConversationId(
            "1476af5a-45a3-495c-afc0-cced3b8aaa0e".into()
        ))
    );
    assert_eq!(resumed.input_tokens, Some(1_778 + 16_715 + 1_780));
    assert_eq!(resumed.context_used_percent, Some(1.719));
}

#[test]
fn a_fork_starts_its_own_totals_at_zero() {
    let fork = sample(&fixture("session_start_fork"));
    assert_eq!(
        fork.conversation,
        Some(ConversationId(
            "01a18be7-a6af-737e-899a-3273df4fb979".into()
        ))
    );
    assert_eq!(
        (fork.input_tokens, fork.output_tokens, fork.cost_usd),
        (Some(0), Some(0), Some(0.0))
    );
    assert_eq!(fork.context_used_percent, Some(3.2));
}

#[test]
fn context_unknown_after_a_compaction_leaves_the_gauge_empty() {
    let compacted = sample(
        r#"{"sessionId":"c1","model":{"id":"local","name":""},"context":{"tokens":null,"contextWindow":32768,"percent":null},"usage":{"input":5,"output":1,"cacheRead":0,"cacheWrite":0,"cost":0}}"#,
    );
    assert_eq!(compacted.context_used_percent, None);
    assert_eq!(compacted.context_window_tokens, Some(32_768));
    assert_eq!(compacted.model.as_deref(), Some("local"));
    assert_eq!(compacted.cost_usd, Some(0.0));
}

#[test]
fn a_tap_without_model_or_context_still_reports_the_totals() {
    let bare = sample(
        r#"{"sessionId":"c1","usage":{"input":5,"output":1,"cacheRead":2,"cacheWrite":3,"cost":0.5}}"#,
    );
    assert_eq!(bare.model, None);
    assert_eq!(bare.context_used_percent, None);
    assert_eq!(bare.context_window_tokens, None);
    assert_eq!(bare.input_tokens, Some(10));
}

#[test]
fn garbage_tap_input_is_an_error() {
    assert!(Pi::default().map_tap("{").is_err());
}
