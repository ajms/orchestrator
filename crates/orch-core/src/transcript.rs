use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TranscriptEntry {
    Prompt {
        text: String,
    },
    Text {
        text: String,
    },
    ToolCall {
        id: String,
        tool: String,
        argument: Option<String>,
    },
    ToolResult {
        id: String,
        text: String,
        error: bool,
    },
}
