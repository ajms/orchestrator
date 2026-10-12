use orch_core::{AgentEvent, ConversationId, UsageSample, UsageWindow};
use serde::Deserialize;

use crate::PayloadError;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct StatusLine {
    session_id: Option<String>,
    model: Model,
    cost: Cost,
    context_window: ContextWindow,
    rate_limits: RateLimits,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Model {
    id: Option<String>,
    display_name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Cost {
    total_cost_usd: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ContextWindow {
    total_input_tokens: Option<u64>,
    total_output_tokens: Option<u64>,
    context_window_size: Option<u64>,
    used_percentage: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RateLimits {
    five_hour: Option<Window>,
    seven_day: Option<Window>,
}

#[derive(Debug, Deserialize)]
struct Window {
    used_percentage: f64,
    resets_at: Option<i64>,
}

impl RateLimits {
    fn named(self) -> Vec<UsageWindow> {
        [
            ("five_hour", "5h", self.five_hour),
            ("seven_day", "7d", self.seven_day),
        ]
        .into_iter()
        .filter_map(|(name, label, window)| {
            window.map(|window| UsageWindow {
                name: name.into(),
                label: label.into(),
                used_percent: window.used_percentage,
                resets_at_unix: window.resets_at,
            })
        })
        .collect()
    }
}

pub(super) fn map_tap(payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
    let line: StatusLine =
        serde_json::from_str(payload).map_err(|err| PayloadError(err.to_string()))?;
    Ok(vec![AgentEvent::UsageSample(UsageSample {
        conversation: line.session_id.map(ConversationId),
        model: line.model.display_name.or(line.model.id),
        context_used_percent: line.context_window.used_percentage,
        context_window_tokens: line.context_window.context_window_size,
        input_tokens: line.context_window.total_input_tokens,
        output_tokens: line.context_window.total_output_tokens,
        cost_usd: line.cost.total_cost_usd,
        windows: line.rate_limits.named(),
    })])
}
