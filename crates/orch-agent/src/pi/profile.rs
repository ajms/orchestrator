use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde_json::{Map, Value};

use crate::profile::{self, create, link, open_root, path_text, remove, staged, swap, write_file};

pub(super) const RESOURCES: [&str; 4] = ["extensions", "skills", "prompts", "themes"];
pub(super) const CREDENTIALS: [&str; 2] = ["auth.json", "models-store.json"];
const LEFT_OUT: [&str; 4] = ["git-worktree-isolation", "sandbox", "notify", "caffeinate"];
const LINKED: [&str; 4] = ["SYSTEM.md", "AGENTS.md", "keybindings.json", "agents"];
const MCP: &str = "mcp.json";
const SETTINGS: &str = "settings.json";
const EXTENSION_DIR: &str = "orch/extension";
const EXTENSION_ENTRY: &str = "index.ts";
const EXTENSION_FILES: [(&str, &str); 5] = [
    (EXTENSION_ENTRY, include_str!("extension/index.ts")),
    ("forward.ts", include_str!("extension/forward.ts")),
    ("outcome.ts", include_str!("extension/outcome.ts")),
    ("prompt.ts", include_str!("extension/prompt.ts")),
    ("transcripts.ts", include_str!("extension/transcripts.ts")),
];

static BUILDING: Mutex<()> = Mutex::new(());

pub(super) fn dirs(lookup: &dyn Fn(&str) -> Option<String>) -> Option<profile::Dirs> {
    profile::Dirs::resolve(lookup, "pi", |home| Some(home.join(".pi/agent")))
}

pub(super) fn extension(profile: &Path) -> PathBuf {
    extension_dir(&profile.join("extensions")).join(EXTENSION_ENTRY)
}

fn extension_dir(extensions: &Path) -> PathBuf {
    extensions.join(EXTENSION_DIR)
}

pub(super) fn build(user: &Path, profile: &Path) -> io::Result<()> {
    let _building = BUILDING.lock().unwrap_or_else(PoisonError::into_inner);
    open_root(profile)?;

    let skills = skills(user);
    let settings = serde_json::to_vec_pretty(&settings(user, &skills.linked_sources)?)?;
    write_file(profile, SETTINGS, &settings)?;

    for name in LINKED.into_iter().chain(CREDENTIALS) {
        let target = user.join(name);
        if target.symlink_metadata().is_ok() {
            link(profile, name, &target)?;
        } else if LINKED.contains(&name) {
            remove(&profile.join(name))?;
        }
    }
    for kind in RESOURCES {
        let staged = staged(profile, kind)?;
        std::fs::create_dir(&staged)?;
        if kind == "extensions" {
            write_extension(&staged)?;
        } else if kind == "skills" {
            for (name, target) in &skills.links {
                std::os::unix::fs::symlink(target, staged.join(name))?;
            }
        }
        swap(profile, kind, &staged)?;
    }

    Ok(())
}

/// Writes the user's `mcp.json` into the profile with `${NAME}` in every
/// string value replaced from `env` (pi reads it without interpolation).
/// Unknown variables stay literal; a missing user file removes the profile's.
pub(super) fn render_mcp(user: &Path, profile: &Path, env: &[(String, String)]) -> io::Result<()> {
    let _building = BUILDING.lock().unwrap_or_else(PoisonError::into_inner);
    let path = user.join(MCP);
    let mut config = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str::<Value>(&text).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {err}", path_text(&path)),
            )
        })?,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return remove(&profile.join(MCP)),
        Err(err) => return Err(err),
    };
    substitute(&mut config, env);
    write_file(profile, MCP, &serde_json::to_vec_pretty(&config)?)
}

fn substitute(value: &mut Value, env: &[(String, String)]) {
    match value {
        Value::String(text) => *text = expand(text, env),
        Value::Array(items) => items.iter_mut().for_each(|item| substitute(item, env)),
        Value::Object(map) => map.values_mut().for_each(|item| substitute(item, env)),
        _ => {}
    }
}

fn expand(text: &str, env: &[(String, String)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let name = after
            .find('}')
            .map(|end| &after[..end])
            .filter(|name| is_name(name));
        let value = name.and_then(|name| env.iter().rev().find(|(key, _)| key == name));
        match (name, value) {
            (Some(name), Some((_, value))) => {
                out.push_str(value);
                rest = &after[name.len() + 1..];
            }
            _ => {
                out.push_str("${");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The user's skills as orch links them into the profile's `skills/`.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Skills {
    /// Link name (the skill directory's name) and target, first source winning.
    pub(super) links: Vec<(String, PathBuf)>,
    /// Sources wholly represented by `links`, left out of `settings.json`.
    linked_sources: Vec<PathBuf>,
}

/// Plans the profile's skill links: `~/.pi/agent/skills`, then the user's
/// `settings.json` `skills` entries. A skill is a directory holding a
/// `SKILL.md`, as pi discovers them; a source that is itself one is linked
/// by its own name. A source with anything else pi would discover (root
/// `*.md` files, nested dirs without `SKILL.md`) stays in `settings.json`.
pub(super) fn skills(user: &Path) -> Skills {
    let mut sources = vec![user.join("skills")];
    let configured = std::fs::read_to_string(user.join(SETTINGS))
        .ok()
        .and_then(|text| serde_json::from_str::<Map<String, Value>>(&text).ok())
        .and_then(|settings| settings.get("skills").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    for entry in configured.iter().filter_map(Value::as_str) {
        if entry.starts_with(['+', '-', '!', '~']) || entry.contains(':') {
            continue;
        }
        sources.push(PathBuf::from(resolve(user, entry)));
    }

    let mut plan = Skills::default();
    let add = |plan: &mut Skills, name: String, target: PathBuf| {
        if !plan.links.iter().any(|(taken, _)| *taken == name) {
            plan.links.push((name, target));
        }
    };
    for source in sources {
        if !source.is_dir() || plan.linked_sources.contains(&source) {
            continue;
        }
        if is_skill(&source) {
            if let Some(name) = source.file_name().and_then(|name| name.to_str()) {
                add(&mut plan, name.into(), source.clone());
                plan.linked_sources.push(source);
            }
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&source) else {
            continue;
        };
        let mut found: Vec<(String, PathBuf)> = Vec::new();
        let mut whole = true;
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(String::from) else {
                whole = false;
                continue;
            };
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            let path = entry.path();
            if path.is_dir() && is_skill(&path) {
                found.push((name, path));
            } else if path.is_dir() || name.ends_with(".md") {
                whole = false;
            }
        }
        found.sort();
        for (name, path) in found {
            add(&mut plan, name, path);
        }
        if whole {
            plan.linked_sources.push(source);
        }
    }
    plan
}

fn is_skill(dir: &Path) -> bool {
    dir.join("SKILL.md").is_file()
}

fn settings(user: &Path, linked: &[PathBuf]) -> io::Result<Map<String, Value>> {
    let path = user.join(SETTINGS);
    let mut settings = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str::<Map<String, Value>>(&text).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {err}", path_text(&path)),
            )
        })?,
        Err(err) if err.kind() == io::ErrorKind::NotFound => Map::new(),
        Err(err) => return Err(err),
    };
    settings.remove("sessionDir");
    for kind in RESOURCES {
        let own = settings
            .get(kind)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|entry| match entry.as_str() {
                Some(text) => Value::from(resolve(user, text)),
                None => entry,
            });
        let mut entries: Vec<Value> = std::iter::once(path_text(&user.join(kind)).into())
            .chain(own)
            .collect();
        if kind == "skills" {
            entries.retain(|entry| {
                entry
                    .as_str()
                    .is_none_or(|text| !linked.iter().any(|dir| path_text(dir) == text))
            });
        }
        if kind == "extensions" {
            entries.extend(LEFT_OUT.map(|name| {
                let entry = user.join(kind).join(name).join("index.ts");
                Value::from(format!("-{}", path_text(&entry)))
            }));
        }
        settings.insert(kind.into(), entries.into());
    }
    Ok(settings)
}

fn resolve(user: &Path, entry: &str) -> String {
    let (sign, path) = match entry.split_at_checked(1) {
        Some((sign @ ("+" | "-"), path)) => (sign, path),
        _ => ("", entry),
    };
    if path.starts_with(['/', '~', '!']) || path.contains(':') {
        return entry.into();
    }
    format!("{sign}{}", path_text(&user.join(path)))
}

fn write_extension(extensions: &Path) -> io::Result<()> {
    let dir = extension_dir(extensions);
    if let Some(parent) = dir.parent() {
        std::fs::create_dir(parent)?;
    }
    std::fs::create_dir(&dir)?;
    for (name, content) in EXTENSION_FILES {
        create(&dir.join(name))?.write_all(content.as_bytes())?;
    }
    Ok(())
}
