use serde_json::Value;

pub(crate) const HOOK_EVENT_FIELD: &str = "orch_hook_event";

pub fn tag_hook_event(payload: &str, event: &str) -> String {
    match serde_json::from_str::<Value>(payload) {
        Ok(Value::Object(mut map)) => {
            map.insert(HOOK_EVENT_FIELD.into(), event.into());
            Value::Object(map).to_string()
        }
        _ => payload.to_string(),
    }
}
