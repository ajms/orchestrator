use serde_json::Value;

const KEY_ARGUMENTS: [&str; 10] = [
    "command",
    "file_path",
    "notebook_path",
    "pattern",
    "url",
    "query",
    "skill",
    "description",
    "prompt",
    "path",
];

pub(crate) fn key_argument(input: &Value) -> Option<String> {
    KEY_ARGUMENTS
        .iter()
        .find_map(|key| input.get(key)?.as_str())
        .map(String::from)
}
