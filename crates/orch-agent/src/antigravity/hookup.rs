use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::Antigravity;
use crate::GUARD_WAIT_SECS;
use crate::hookup::{AgentHookup, FileEdit, HookupError, HookupState, Lookup, edit, read};
use crate::shell::quote;

const HOOK_NAME: &str = "orch";
const TOOL_EVENTS: [&str; 2] = ["PreToolUse", "PostToolUse"];
const LOOP_EVENTS: [&str; 3] = ["PreInvocation", "PostInvocation", "Stop"];
const GUARD_EVENT: &str = "PreToolUse";
const STATUS_LINE: &str = "statusLine";

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

fn hook_entry(orch_program: &str) -> Value {
    let hook = json!({ "type": "command", "command": orch_command(orch_program, "hook") });
    let mut entry = Map::new();
    for event in TOOL_EVENTS {
        let mut hook = hook.clone();
        if event == GUARD_EVENT {
            hook["timeout"] = json!(GUARD_WAIT_SECS);
        }
        entry.insert(event.into(), json!([{ "matcher": "*", "hooks": [hook] }]));
    }
    for event in LOOP_EVENTS {
        entry.insert(event.into(), json!([hook]));
    }
    Value::Object(entry)
}

fn is_orch_tap(status_line: Option<&Value>) -> bool {
    status_line
        .and_then(|line| line["command"].as_str())
        .is_some_and(|command| command.ends_with(&format!(" tap --agent {}", Antigravity::NAME)))
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

struct Json {
    path: PathBuf,
    before: Option<String>,
    map: Map<String, Value>,
}

impl Json {
    fn load(path: PathBuf) -> Result<Self, HookupError> {
        let before = read(&path)?;
        let map = parse(&path, before.as_deref())?;
        Ok(Self { path, before, map })
    }

    fn edit(self, map: Map<String, Value>) -> Option<FileEdit> {
        let after = match map == self.map {
            true => self.before.clone(),
            false => Some(render(map)),
        };
        edit(self.path, self.before, after)
    }
}

pub(super) fn saved_status_line_command(lookup: Lookup) -> Option<String> {
    let record = Json::load(Files::locate(lookup).ok()?.record).ok()?;
    let command = record.map.get(STATUS_LINE)?["command"].as_str()?.trim();
    (!command.is_empty()).then(|| command.to_string())
}

impl AgentHookup for AntigravityHookup {
    fn install(&self, orch_program: &str, lookup: Lookup) -> Result<Vec<FileEdit>, HookupError> {
        let files = Files::locate(lookup)?;
        let hooks = Json::load(files.hooks)?;
        let settings = Json::load(files.settings)?;
        let record = Json::load(files.record)?;

        let mut new_hooks = hooks.map.clone();
        new_hooks.insert(HOOK_NAME.into(), hook_entry(orch_program));

        let previous = settings.map.get(STATUS_LINE).cloned();
        let record_edit = match is_orch_tap(previous.as_ref()) && record.before.is_some() {
            true => None,
            false => {
                let saved = previous.filter(|line| !is_orch_tap(Some(line)));
                let mut new_record = Map::new();
                new_record.insert(STATUS_LINE.into(), saved.unwrap_or(Value::Null));
                record.edit(new_record)
            }
        };
        let mut new_settings = settings.map.clone();
        new_settings.insert(
            STATUS_LINE.into(),
            json!({ "type": "command", "command": orch_command(orch_program, "tap") }),
        );

        Ok([
            hooks.edit(new_hooks),
            settings.edit(new_settings),
            record_edit,
        ]
        .into_iter()
        .flatten()
        .collect())
    }

    fn uninstall(&self, lookup: Lookup) -> Result<Vec<FileEdit>, HookupError> {
        let files = Files::locate(lookup)?;
        let hooks = Json::load(files.hooks)?;
        let settings = Json::load(files.settings)?;
        let record = Json::load(files.record)?;

        let mut new_hooks = hooks.map.clone();
        new_hooks.remove(HOOK_NAME);

        let mut new_settings = settings.map.clone();
        if is_orch_tap(settings.map.get(STATUS_LINE)) {
            match record.map.get(STATUS_LINE).filter(|line| !line.is_null()) {
                Some(saved) => new_settings.insert(STATUS_LINE.into(), saved.clone()),
                None => new_settings.remove(STATUS_LINE),
            };
        }

        Ok([
            hooks.edit(new_hooks),
            settings.edit(new_settings),
            edit(record.path, record.before, None),
        ]
        .into_iter()
        .flatten()
        .collect())
    }

    fn state(&self, orch_program: &str, lookup: Lookup) -> HookupState {
        let loaded = Files::locate(lookup).and_then(|files| {
            Ok((
                Json::load(files.hooks)?,
                Json::load(files.settings)?,
                Json::load(files.record)?,
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
            || hooks.map.contains_key(HOOK_NAME)
            || is_orch_tap(settings.map.get(STATUS_LINE));
        match (problems.is_empty(), traces) {
            (true, _) => HookupState::Installed,
            (false, false) => HookupState::Missing,
            (false, true) => HookupState::Broken(problems.join("; ")),
        }
    }
}
