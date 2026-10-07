use serde_json::{Value, json};

const SUCCESS: &str = "SUCCESS";

pub(crate) fn encode(prompt: &str) -> String {
    let line = json!({
        "event": "user",
        "message": { "content": [{ "type": "text", "text": prompt }] },
    });
    format!("{line}\n")
}

pub(crate) fn decode(stdout: &str) -> Result<Option<String>, String> {
    let Some(result) = stdout
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|event| event["event"] == "result")
        .map(|event| event["result"].clone())
    else {
        return Ok(None);
    };
    let status = result["status"].as_str().unwrap_or_default();
    if status != SUCCESS {
        let error = match &result["error"] {
            Value::Null => format!("agy ended with status {status:?}"),
            Value::String(error) => error.clone(),
            error => error.to_string(),
        };
        return Err(error);
    }
    Ok(Some(result["response"].as_str().unwrap_or_default().into()))
}
