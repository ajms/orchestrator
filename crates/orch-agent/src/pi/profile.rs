use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde_json::{Map, Value};

pub(super) const RESOURCES: [&str; 4] = ["extensions", "skills", "prompts", "themes"];
pub(super) const CREDENTIALS: [&str; 2] = ["auth.json", "models-store.json"];
const LEFT_OUT: [&str; 4] = ["git-worktree-isolation", "sandbox", "notify", "caffeinate"];
const LINKED: [&str; 4] = ["SYSTEM.md", "AGENTS.md", "keybindings.json", "agents"];
const SETTINGS: &str = "settings.json";
const STAGED: &str = ".orch-new";
const RETIRED: &str = ".orch-old";

static BUILDING: Mutex<()> = Mutex::new(());

pub(super) fn home(lookup: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    lookup("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

pub(super) fn user_dir(lookup: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    home(lookup).map(|home| home.join(".pi/agent"))
}

pub(super) fn profile_dir(lookup: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let state = lookup("XDG_STATE_HOME")
        .filter(|state| !state.is_empty())
        .map(PathBuf::from)
        .or_else(|| home(lookup).map(|home| home.join(".local/state")))?;
    Some(state.join("orchestrator/agent-profiles/pi"))
}

pub(super) fn build(user: &Path, profile: &Path) -> io::Result<()> {
    let _building = BUILDING.lock().unwrap_or_else(PoisonError::into_inner);
    std::fs::create_dir_all(profile)?;
    for dir in [Some(profile), profile.parent()].into_iter().flatten() {
        if !dir.symlink_metadata()?.is_dir() {
            return Err(io::Error::other(format!(
                "{} is not a directory",
                dir.display()
            )));
        }
    }
    std::fs::set_permissions(profile, std::fs::Permissions::from_mode(0o700))?;

    let settings = serde_json::to_vec_pretty(&settings(user)?)?;
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
        swap(profile, kind, &staged)?;
    }

    Ok(())
}

fn settings(user: &Path) -> io::Result<Map<String, Value>> {
    let path = user.join(SETTINGS);
    let mut settings = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str::<Map<String, Value>>(&text).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}: {err}", path.display()),
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
        let mut entries: Vec<Value> = std::iter::once(text(&user.join(kind)).into())
            .chain(own)
            .collect();
        if kind == "extensions" {
            entries.extend(LEFT_OUT.map(|name| {
                let entry = user.join(kind).join(name).join("index.ts");
                Value::from(format!("-{}", text(&entry)))
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
    format!("{sign}{}", text(&user.join(path)))
}

fn write_file(profile: &Path, name: &str, content: &[u8]) -> io::Result<()> {
    let staged = staged(profile, name)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&staged)?;
    file.write_all(content)?;
    swap(profile, name, &staged)
}

fn link(profile: &Path, name: &str, target: &Path) -> io::Result<()> {
    if std::fs::read_link(profile.join(name)).is_ok_and(|current| current == target) {
        return Ok(());
    }
    let staged = staged(profile, name)?;
    std::os::unix::fs::symlink(target, &staged)?;
    swap(profile, name, &staged)
}

fn staged(profile: &Path, name: &str) -> io::Result<PathBuf> {
    let staged = profile.join(format!("{name}{STAGED}"));
    remove(&staged)?;
    Ok(staged)
}

fn swap(profile: &Path, name: &str, staged: &Path) -> io::Result<()> {
    let path = profile.join(name);
    match std::fs::rename(staged, &path) {
        Err(_) if path.symlink_metadata().is_ok() => {
            let retired = profile.join(format!("{name}{RETIRED}"));
            remove(&retired)?;
            std::fs::rename(&path, &retired)?;
            std::fs::rename(staged, &path)?;
            remove(&retired)
        }
        renamed => renamed,
    }
}

fn remove(path: &Path) -> io::Result<()> {
    let removed = match path.symlink_metadata() {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(err) => Err(err),
    };
    match removed {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
