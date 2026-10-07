use orch_core::{AgentEvent, GuardedAction};
use serde_json::Value;

const WRITE_TOOLS: [&str; 8] = [
    "write_to_file",
    "write_file",
    "create_file",
    "edit_file",
    "replace_file_content",
    "multi_replace_file_content",
    "delete_file",
    "edit_notebook",
];
const PATH_FIELDS: [&str; 5] = [
    "TargetFile",
    "AbsolutePath",
    "FilePath",
    "File",
    "NotebookPath",
];
pub(super) const MCP_PREFIX: &str = "mcp_";

pub(super) fn guard_check(tool: &str, args: &Value) -> Option<AgentEvent> {
    let field = |key: &str| args.get(key).and_then(Value::as_str).map(String::from);
    let or_unreadable = |action: Option<GuardedAction>| action.unwrap_or(GuardedAction::Unreadable);
    let (action, cwd) = match tool {
        _ if WRITE_TOOLS.contains(&tool) => {
            let path = PATH_FIELDS.iter().find_map(|key| field(key));
            (
                or_unreadable(path.map(|path| GuardedAction::WriteFile { path })),
                None,
            )
        }
        "run_command" => {
            let command = field("CommandLine").map(|command| GuardedAction::Shell { command });
            (or_unreadable(command), field("Cwd"))
        }
        "send_command_input" => {
            let command = field("Input").map(|command| GuardedAction::Shell { command });
            (or_unreadable(command), None)
        }
        _ if is_external(tool) => (GuardedAction::ExternalTool { name: tool.into() }, None),
        _ => return None,
    };
    Some(AgentEvent::GuardCheck {
        tool: tool.into(),
        action,
        cwd,
    })
}

fn is_external(tool: &str) -> bool {
    tool.starts_with(MCP_PREFIX) || tool.split('_').any(|part| part == "browser")
}
