use orch_agent::{AgentAdapter, Antigravity};
use orch_core::{AgentEvent, ConversationId, PermissionMode, UsageSample, UsageWindow};

const CONVERSATION: &str = "3c1e9a40-7d52-4b8e-a6f1-2d9b0c4e7a13";

fn all_events(name: &str) -> Vec<AgentEvent> {
    let path = format!(
        "{}/tests/fixtures/antigravity/statusline/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let payload = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{path}: {err}"));
    Antigravity::default()
        .map_tap(&payload)
        .unwrap_or_else(|err| panic!("{name}: {err:?}"))
}

fn events(name: &str) -> Vec<AgentEvent> {
    all_events(name)
        .into_iter()
        .filter(|event| !matches!(event, AgentEvent::UsageSample(_)))
        .collect()
}

fn usage(name: &str) -> UsageSample {
    let samples: Vec<_> = all_events(name)
        .into_iter()
        .filter_map(|event| match event {
            AgentEvent::UsageSample(sample) => Some(sample),
            _ => None,
        })
        .collect();
    match samples.as_slice() {
        [sample] => sample.clone(),
        other => panic!("one usage sample expected, got {other:?}"),
    }
}

fn weekly(name: &str, label: &str, used_percent: f64) -> UsageWindow {
    UsageWindow {
        name: name.into(),
        label: label.into(),
        used_percent,
        resets_at_unix: Some(1_791_763_200),
    }
}

fn conversation() -> AgentEvent {
    AgentEvent::ConversationChanged {
        id: ConversationId(CONVERSATION.into()),
    }
}

fn mode(mode: PermissionMode) -> AgentEvent {
    AgentEvent::ModeChanged { mode }
}

#[test]
fn the_trust_screen_reports_neither_needs_input_nor_readiness_nor_usage() {
    assert_eq!(all_events("trust_screen"), [AgentEvent::PermissionCleared]);
}

#[test]
fn an_idle_line_before_agy_has_a_conversation_is_not_readiness() {
    let line = r#"{"conversation_id":"","agent_state":"idle"}"#;
    let events = Antigravity::default().map_tap(line).unwrap();
    assert!(!events.contains(&AgentEvent::AwaitingPrompt), "{events:?}");
}

#[test]
fn an_idle_line_reports_the_conversation_the_default_mode_and_readiness() {
    assert_eq!(
        events("idle"),
        [
            conversation(),
            mode(PermissionMode::Default),
            AgentEvent::PermissionCleared,
            AgentEvent::AwaitingPrompt,
        ]
    );
}

#[test]
fn a_pending_tool_confirmation_needs_input_in_the_cycled_mode() {
    assert_eq!(
        events("tool_confirmation_accept_edits"),
        [
            conversation(),
            mode(PermissionMode::AcceptEdits),
            AgentEvent::PermissionRequested,
        ]
    );
}

#[test]
fn a_working_line_clears_a_permission_prompt_and_tracks_plan_mode() {
    assert_eq!(
        events("working_plan"),
        [
            conversation(),
            mode(PermissionMode::Plan),
            AgentEvent::PermissionCleared,
        ]
    );
}

#[test]
fn an_idle_line_samples_usage_with_one_window_per_quota_pool_and_no_cost() {
    let sample = usage("idle");
    assert_eq!(
        sample,
        UsageSample {
            conversation: Some(ConversationId(CONVERSATION.into())),
            model: Some("Gemini 3.8 Flash".into()),
            context_used_percent: Some(1.8),
            context_window_tokens: Some(1_048_576),
            input_tokens: Some(18_342),
            output_tokens: Some(912),
            cost_usd: None,
            windows: vec![
                weekly("gemini-weekly", "gemini-wk", 17.0),
                weekly("3p-weekly", "3p-wk", 0.0),
            ],
        }
    );
}

#[test]
fn an_exhausted_pool_omits_its_remaining_fraction_and_is_fully_used() {
    assert_eq!(
        usage("quota_exhausted").windows,
        [
            weekly("gemini-weekly", "gemini-wk", 100.0),
            UsageWindow {
                resets_at_unix: None,
                ..weekly("3p-weekly", "3p-wk", 60.0)
            },
        ]
    );
}

fn windows_of(quota: &str) -> Vec<UsageWindow> {
    let line = format!(r#"{{"quota":{quota}}}"#);
    let events = Antigravity::default().map_tap(&line).unwrap();
    match events.as_slice() {
        [AgentEvent::UsageSample(sample), ..] => sample.windows.clone(),
        other => panic!("a usage sample expected, got {other:?}"),
    }
}

fn unscheduled(name: &str, label: &str, used_percent: f64) -> UsageWindow {
    UsageWindow {
        resets_at_unix: None,
        ..weekly(name, label, used_percent)
    }
}

#[test]
fn a_pool_orch_does_not_know_is_labelled_by_its_name() {
    assert_eq!(
        windows_of(r#"{"gemini-daily":{"remaining_fraction":0.5}}"#),
        [unscheduled("gemini-daily", "gemini-daily", 50.0)]
    );
}

#[test]
fn a_null_remaining_fraction_keeps_its_pool_as_fully_used() {
    assert_eq!(
        windows_of(r#"{"gemini-weekly":{"remaining_fraction":null}}"#),
        [unscheduled("gemini-weekly", "gemini-wk", 100.0)]
    );
}

#[test]
fn a_remaining_fraction_outside_zero_to_one_stays_within_zero_to_a_hundred_percent() {
    assert_eq!(
        windows_of(
            r#"{"gemini-weekly":{"remaining_fraction":1.2},"3p-weekly":{"remaining_fraction":-0.1}}"#
        ),
        [
            unscheduled("gemini-weekly", "gemini-wk", 0.0),
            unscheduled("3p-weekly", "3p-wk", 100.0),
        ]
    );
}

#[test]
fn garbage_statusline_input_is_an_error() {
    assert!(Antigravity::default().map_tap("not json").is_err());
}
