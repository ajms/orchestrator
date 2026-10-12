use std::path::{Path, PathBuf};

use serde_json::Value;

const MANAGED_SETTINGS: &str = "/etc/claude-code/managed-settings.json";
const MANAGED_SETTINGS_ENV: &str = "ORCH_CLAUDE_MANAGED_SETTINGS";

pub(super) fn user_statusline_command(
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    settings_files(cwd, lookup)
        .iter()
        .find_map(|path| statusline_command(path))
}

pub(super) fn config_dir(lookup: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let var = |key| lookup(key).filter(|value: &String| !value.is_empty());
    var("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|home| Path::new(&home).join(".claude")))
}

fn settings_files(cwd: &Path, lookup: impl Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let managed = lookup(MANAGED_SETTINGS_ENV)
        .filter(|value| !value.is_empty())
        .map_or_else(|| MANAGED_SETTINGS.into(), PathBuf::from);
    let user = config_dir(&lookup).map(|dir| dir.join("settings.json"));
    [
        Some(managed),
        Some(cwd.join(".claude/settings.local.json")),
        Some(cwd.join(".claude/settings.json")),
        user,
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn statusline_command(path: &Path) -> Option<String> {
    let settings: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let command = settings["statusLine"]["command"].as_str()?.trim();
    (!command.is_empty()).then(|| command.to_string())
}
