use orch_agent::{AgentAdapter, ClaudeCode};
use orch_core::{AgentEvent, ConversationId, RateLimit, UsageSample};

fn usage(name: &str) -> Vec<AgentEvent> {
    let path = format!(
        "{}/tests/fixtures/statusline/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let payload = std::fs::read_to_string(&path).unwrap();
    ClaudeCode::default()
        .map_tap(&payload)
        .unwrap_or_else(|err| panic!("{name}: {err:?}"))
}

fn only_sample(events: Vec<AgentEvent>) -> UsageSample {
    match events.as_slice() {
        [AgentEvent::UsageSample(sample)] => sample.clone(),
        other => panic!("one usage sample expected, got {other:?}"),
    }
}

#[test]
fn a_subscribers_statusline_yields_context_cost_and_rate_limits() {
    assert_eq!(
        usage("subscriber"),
        vec![AgentEvent::UsageSample(UsageSample {
            conversation: Some(ConversationId(
                "5f0c6a52-6a3e-4d3b-9d7e-0b1f8c1e2a90".into()
            )),
            model: Some("Opus 5.5".into()),
            context_used_percent: Some(42.0),
            context_window_tokens: Some(200_000),
            input_tokens: Some(84_211),
            output_tokens: Some(3_107),
            cost_usd: Some(1.8342),
            limits: vec![
                RateLimit {
                    name: "five_hour".into(),
                    label: "5h".into(),
                    used_percent: 83.5,
                    resets_at_unix: Some(1_790_592_000),
                },
                RateLimit {
                    name: "seven_day".into(),
                    label: "7d".into(),
                    used_percent: 41.2,
                    resets_at_unix: Some(1_791_014_400),
                },
            ],
        })]
    );
}

#[test]
fn fields_missing_before_the_first_response_stay_unknown() {
    let sample = only_sample(usage("before_first_response"));
    assert_eq!(sample.context_used_percent, None);
    assert_eq!(sample.context_window_tokens, Some(200_000));
    assert_eq!(sample.cost_usd, Some(0.0));
    assert_eq!(sample.limits, []);
}

#[test]
fn each_rate_limit_window_may_be_absent_on_its_own() {
    let sample = only_sample(usage("seven_day_only"));
    assert_eq!(sample.model.as_deref(), Some("claude-sonnet-5"));
    let windows: Vec<_> = sample
        .limits
        .iter()
        .map(|limit| (limit.label.as_str(), limit.used_percent))
        .collect();
    assert_eq!(windows, [("7d", 96.4)]);
}

#[test]
fn garbage_statusline_input_is_an_error() {
    assert!(ClaudeCode::default().map_tap("{").is_err());
}

#[test]
fn each_sample_names_its_conversation_so_usage_survives_clear() {
    let sample = only_sample(usage("seven_day_only"));
    assert_eq!(
        sample.conversation,
        Some(ConversationId(
            "0d3f8a21-7b6c-4e5d-9a8b-1c2d3e4f5a6b".into()
        ))
    );
}
