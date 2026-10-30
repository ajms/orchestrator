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
const EXTENSION_DIR: &str = "orch/extension";
const EXTENSION_ENTRY: &str = "index.ts";
const EXTENSION_FILES: [(&str, &str); 2] = [
    (EXTENSION_ENTRY, include_str!("extension/index.ts")),
    ("forward.ts", include_str!("extension/forward.ts")),
];
const STAGED: &str = ".orch-new";
const RETIRED: &str = ".orch-old";

static BUILDING: Mutex<()> = Mutex::new(());

pub(super) struct Dirs {
    pub(super) home: PathBuf,
    pub(super) user: PathBuf,
    pub(super) profile: PathBuf,
}

impl Dirs {
    pub(super) fn resolve(lookup: &dyn Fn(&str) -> Option<String>) -> Option<Self> {
        let set = |key: &str| lookup(key).filter(|value| !value.is_empty());
        let home = PathBuf::from(set("HOME")?);
        let state = set("XDG_STATE_HOME").map_or_else(|| home.join(".local/state"), PathBuf::from);
        Some(Self {
            user: home.join(".pi/agent"),
            profile: state.join("orchestrator/agent-profiles/pi"),
            home,
        })
    }
}

pub(super) fn extension(profile: &Path) -> PathBuf {
    extension_dir(&profile.join("extensions")).join(EXTENSION_ENTRY)
}

fn extension_dir(extensions: &Path) -> PathBuf {
    extensions.join(EXTENSION_DIR)
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
        if kind == "extensions" {
            write_extension(&staged)?;
        }
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

fn create(path: &Path) -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

fn write_file(profile: &Path, name: &str, content: &[u8]) -> io::Result<()> {
    let staged = staged(profile, name)?;
    create(&staged)?.write_all(content)?;
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

pub(super) fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
