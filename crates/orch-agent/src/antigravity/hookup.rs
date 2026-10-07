use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::{Antigravity, GUARD_EVENT};
use crate::GUARD_WAIT_SECS;
use crate::hookup::{
    AgentHookup, FileEdit, HookupError, HookupState, Lookup, file_edit, read_optional,
};
use crate::shell::quote;

const HOOK_NAME: &str = "orch";
const TOOL_EVENTS: [&str; 2] = ["PreToolUse", "PostToolUse"];
const LOOP_EVENTS: [&str; 3] = ["PreInvocation", "PostInvocation", "Stop"];
const STATUS_LINE: &str = "statusLine";
const FILES: &str = "files";
const HOOKS: &str = "hooks";
const SETTINGS: &str = "settings";
const ORIGINAL: &str = "original";
const INSTALLED: &str = "installed";

pub(crate) struct AntigravityHookup;

struct Files {
    hooks: PathBuf,
    settings: PathBuf,
    record: PathBuf,
}

impl Files {
    fn locate(lookup: Lookup) -> Result<Self, HookupError> {
        let var = |key| lookup(key).filter(|value: &String| !value.is_empty());
        let home = var("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| HookupError("HOME is not set".into()))?;
        let state = var("XDG_STATE_HOME").map_or_else(|| home.join(".local/state"), PathBuf::from);
        Ok(Self {
            hooks: home.join(".gemini/config/hooks.json"),
            settings: home.join(".gemini/antigravity-cli/settings.json"),
            record: state.join("orchestrator/hookups/antigravity.json"),
        })
    }
}

fn orch_command(orch_program: &str, subcommand: &str) -> String {
    format!(
        "{} {subcommand} --agent {}",
        quote(orch_program),
        Antigravity::NAME
    )
}

fn is_orch_command(command: &str, subcommand: &str) -> bool {
    let orch = format!(" {subcommand} --agent {}", Antigravity::NAME);
    command.ends_with(&orch) || command.contains(&format!("{orch} --event "))
}

fn hook_entry(orch_program: &str) -> Value {
    let hook = |event: &str| {
        let command = format!("{} --event {event}", orch_command(orch_program, "hook"));
        json!({ "type": "command", "command": command })
    };
    let mut entry = Map::new();
    for event in TOOL_EVENTS {
        let mut hook = hook(event);
        if event == GUARD_EVENT {
            hook["timeout"] = json!(GUARD_WAIT_SECS);
        }
        entry.insert(event.into(), json!([{ "matcher": "*", "hooks": [hook] }]));
    }
    for event in LOOP_EVENTS {
        entry.insert(event.into(), json!([hook(event)]));
    }
    Value::Object(entry)
}

fn commands<'a>(value: &'a Value, found: &mut Vec<&'a Value>) {
    match value {
        Value::Object(map) => map.iter().for_each(|(key, value)| match key.as_str() {
            "command" => found.push(value),
            _ => commands(value, found),
        }),
        Value::Array(items) => items.iter().for_each(|item| commands(item, found)),
        _ => {}
    }
}

fn is_orch_hook(entry: &Value) -> bool {
    let mut found = Vec::new();
    commands(entry, &mut found);
    !found.is_empty()
        && found.iter().all(|command| {
            command
                .as_str()
                .is_some_and(|command| is_orch_command(command, "hook"))
        })
}

fn is_untagged(entry: &Value) -> bool {
    let mut found = Vec::new();
    commands(entry, &mut found);
    found
        .iter()
        .filter_map(|command| command.as_str())
        .any(|command| !command.contains(" --event "))
}

fn is_orch_tap(status_line: Option<&Value>) -> bool {
    status_line
        .and_then(|line| line["command"].as_str())
        .is_some_and(|command| is_orch_command(command, "tap"))
}

fn parse(path: &Path, contents: Option<&str>) -> Result<Map<String, Value>, HookupError> {
    let contents = contents.unwrap_or_default();
    if contents.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str(contents) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(HookupError(format!(
            "{} is not a JSON object",
            path.display()
        ))),
    }
}

fn render(map: Map<String, Value>) -> String {
    let mut rendered = serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_default();
    rendered.push('\n');
    rendered
}

struct JsonFile {
    path: PathBuf,
    before: Option<String>,
    map: Map<String, Value>,
}

impl JsonFile {
    fn load(path: PathBuf) -> Result<Self, HookupError> {
        let before = read_optional(&path)?;
        let map = parse(&path, before.as_deref())?;
        Ok(Self { path, before, map })
    }

    fn after(&self, map: Map<String, Value>) -> Option<String> {
        match map == self.map {
            true => self.before.clone(),
            false => Some(render(map)),
        }
    }

    fn file_edit(self, after: Option<String>) -> Option<FileEdit> {
        file_edit(self.path, self.before, after)
    }
}

fn restore_point(
    record: &JsonFile,
    role: &str,
    file: &JsonFile,
    has_orch_parts: bool,
) -> Option<Value> {
    if !has_orch_parts {
        return Some(json!(file.before));
    }
    let entry = record.map.get(FILES)?.get(role)?;
    (entry[INSTALLED].as_str() == file.before.as_deref()).then(|| entry[ORIGINAL].clone())
}

fn exact_restore(record: &JsonFile, role: &str, file: &JsonFile) -> Option<Option<String>> {
    let entry = record.map.get(FILES)?.get(role)?;
    let installed = entry[INSTALLED].as_str()?;
    (Some(installed) == file.before.as_deref())
        .then(|| entry[ORIGINAL].as_str().map(str::to_string))
}

pub(super) fn saved_status_line_command(lookup: Lookup) -> Option<String> {
    let record = JsonFile::load(Files::locate(lookup).ok()?.record).ok()?;
    let command = record.map.get(STATUS_LINE)?["command"].as_str()?.trim();
    (!command.is_empty()).then(|| command.to_string())
}

impl AgentHookup for AntigravityHookup {
    fn install(&self, orch_program: &str, lookup: Lookup) -> Result<Vec<FileEdit>, HookupError> {
        let files = Files::locate(lookup)?;
        let hooks = JsonFile::load(files.hooks)?;
        let settings = JsonFile::load(files.settings)?;
        let record = JsonFile::load(files.record)?;

        let existing_hook = hooks.map.get(HOOK_NAME);
        if existing_hook.is_some_and(|entry| !is_orch_hook(entry)) {
            return Err(HookupError(format!(
                "{} already has a \"{HOOK_NAME}\" hook that orch did not write; \
                 rename or remove it, then install again",
                hooks.path.display()
            )));
        }
        let mut new_hooks = hooks.map.clone();
        new_hooks.insert(HOOK_NAME.into(), hook_entry(orch_program));

        let previous = settings.map.get(STATUS_LINE);
        let saved = match is_orch_tap(previous) && record.before.is_some() {
            true => record.map.get(STATUS_LINE).cloned(),
            false => previous.filter(|line| !is_orch_tap(Some(line))).cloned(),
        };
        let mut new_settings = settings.map.clone();
        new_settings.insert(
            STATUS_LINE.into(),
            json!({ "type": "command", "command": orch_command(orch_program, "tap") }),
        );

        let hooks_after = hooks.after(new_hooks);
        let settings_after = settings.after(new_settings);
        let mut restore = Map::new();
        if let Some(original) = restore_point(&record, HOOKS, &hooks, existing_hook.is_some()) {
            restore.insert(
                HOOKS.into(),
                json!({ ORIGINAL: original, INSTALLED: hooks_after }),
            );
        }
        if let Some(original) = restore_point(&record, SETTINGS, &settings, is_orch_tap(previous)) {
            restore.insert(
                SETTINGS.into(),
                json!({ ORIGINAL: original, INSTALLED: settings_after }),
            );
        }
        let mut new_record = Map::new();
        new_record.insert(STATUS_LINE.into(), saved.unwrap_or(Value::Null));
        new_record.insert(FILES.into(), Value::Object(restore));
        let record_after = record.after(new_record);

        Ok([
            record.file_edit(record_after),
            hooks.file_edit(hooks_after),
            settings.file_edit(settings_after),
        ]
        .into_iter()
        .flatten()
        .collect())
    }

    fn uninstall(&self, lookup: Lookup) -> Result<Vec<FileEdit>, HookupError> {
        let files = Files::locate(lookup)?;
        let hooks = JsonFile::load(files.hooks)?;
        let settings = JsonFile::load(files.settings)?;
        let record = JsonFile::load(files.record)?;

        let hooks_after = exact_restore(&record, HOOKS, &hooks).unwrap_or_else(|| {
            let mut new_hooks = hooks.map.clone();
            if new_hooks.get(HOOK_NAME).is_some_and(is_orch_hook) {
                new_hooks.remove(HOOK_NAME);
            }
            hooks.after(new_hooks)
        });
        let settings_after = exact_restore(&record, SETTINGS, &settings).unwrap_or_else(|| {
            let mut new_settings = settings.map.clone();
            if is_orch_tap(settings.map.get(STATUS_LINE)) {
                match record.map.get(STATUS_LINE).filter(|line| !line.is_null()) {
                    Some(saved) => new_settings.insert(STATUS_LINE.into(), saved.clone()),
                    None => new_settings.remove(STATUS_LINE),
                };
            }
            settings.after(new_settings)
        });

        Ok([
            hooks.file_edit(hooks_after),
            settings.file_edit(settings_after),
            record.file_edit(None),
        ]
        .into_iter()
        .flatten()
        .collect())
    }

    fn state(&self, orch_program: &str, lookup: Lookup) -> HookupState {
        let loaded = Files::locate(lookup).and_then(|files| {
            Ok((
                JsonFile::load(files.hooks)?,
                JsonFile::load(files.settings)?,
                JsonFile::load(files.record)?,
            ))
        });
        let (hooks, settings, record) = match loaded {
            Ok(loaded) => loaded,
            Err(err) => return HookupState::Broken(err.0),
        };
        let mut problems = Vec::new();
        match hooks.map.get(HOOK_NAME) {
            None => problems.push(format!(
                "the orch hook is missing from {}",
                hooks.path.display()
            )),
            Some(entry) if is_orch_hook(entry) && is_untagged(entry) => problems.push(format!(
                "the orch hook in {} has commands without `--event` tags; \
                 rerun `orch agent install {}`",
                hooks.path.display(),
                Antigravity::NAME
            )),
            Some(entry) if *entry != hook_entry(orch_program) => problems.push(format!(
                "the orch hook in {} is not the one this orch installs",
                hooks.path.display()
            )),
            Some(_) => {}
        }
        let tap = settings.map.get(STATUS_LINE).map(|line| &line["command"]);
        if tap.and_then(Value::as_str) != Some(&orch_command(orch_program, "tap")) {
            problems.push(format!(
                "the statusLine in {} is not orch's tap",
                settings.path.display()
            ));
        }
        let traces = record.before.is_some()
            || hooks.map.get(HOOK_NAME).is_some_and(is_orch_hook)
            || is_orch_tap(settings.map.get(STATUS_LINE));
        match (problems.is_empty(), traces) {
            (true, _) => HookupState::Installed,
            (false, false) => HookupState::Missing,
            (false, true) => HookupState::Broken(problems.join("; ")),
        }
    }
}
