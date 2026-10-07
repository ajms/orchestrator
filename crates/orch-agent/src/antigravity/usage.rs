use orch_core::{ConversationId, UsageSample, UsageWindow};
use serde::Deserialize;
use serde_json::{Map, Value};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const POOL_LABELS: [(&str, &str); 2] = [("gemini-weekly", "gemini-wk"), ("3p-weekly", "3p-wk")];

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct Usage {
    model: Option<Model>,
    context_window: Option<ContextWindow>,
    quota: Option<Map<String, Value>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Model {
    display_name: Option<String>,
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
struct Pool {
    remaining_fraction: f64,
    reset_time: Option<String>,
}

impl Usage {
    pub(super) fn sample(self, conversation: Option<ConversationId>) -> UsageSample {
        let context = self.context_window.unwrap_or_default();
        UsageSample {
            conversation,
            model: self.model.and_then(|model| model.display_name),
            context_used_percent: context.used_percentage,
            context_window_tokens: context.context_window_size,
            input_tokens: context.total_input_tokens,
            output_tokens: context.total_output_tokens,
            cost_usd: None,
            windows: self
                .quota
                .unwrap_or_default()
                .into_iter()
                .filter_map(window)
                .collect(),
        }
    }
}

fn label(name: &str) -> &str {
    POOL_LABELS
        .iter()
        .find(|(pool, _)| *pool == name)
        .map_or(name, |(_, label)| label)
}

fn window((name, pool): (String, Value)) -> Option<UsageWindow> {
    let pool: Pool = serde_json::from_value(pool).ok()?;
    Some(UsageWindow {
        label: label(&name).into(),
        used_percent: (100.0 - pool.remaining_fraction * 100.0).clamp(0.0, 100.0),
        resets_at_unix: pool
            .reset_time
            .and_then(|at| OffsetDateTime::parse(&at, &Rfc3339).ok())
            .map(OffsetDateTime::unix_timestamp),
        name,
    })
}
