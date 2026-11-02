use orch_core::{AgentEvent, ConversationId, UsageSample};
use serde::Deserialize;

use crate::PayloadError;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tap {
    session_id: String,
    model: Option<Model>,
    context: Option<Context>,
    last_response: Tokens,
    cost: f64,
}

#[derive(Debug, Deserialize)]
struct Model {
    id: String,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Context {
    context_window: Option<u64>,
    percent: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tokens {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
}

pub(super) fn map_tap(payload: &str) -> Result<Vec<AgentEvent>, PayloadError> {
    let tap: Tap = serde_json::from_str(payload).map_err(|err| PayloadError(err.to_string()))?;
    let model = tap.model.map(|model| {
        model
            .name
            .filter(|name| !name.is_empty())
            .unwrap_or(model.id)
    });
    let last = tap.last_response;
    Ok(vec![AgentEvent::UsageSample(UsageSample {
        conversation: Some(ConversationId(tap.session_id)),
        model,
        context_used_percent: tap.context.as_ref().and_then(|context| context.percent),
        context_window_tokens: tap.context.and_then(|context| context.context_window),
        input_tokens: Some(last.input + last.cache_read + last.cache_write),
        output_tokens: Some(last.output),
        cost_usd: Some(tap.cost),
        five_hour: None,
        seven_day: None,
    })])
}
