use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use serde_json::{Map, Value, json};

use super::CONFIG_CARVE_OUTS;

/// The credential file orch links (not copies) from the real `~/.claude` so
/// that Claude's token refresh keeps working, the same way pi links `auth.json`.
pub(super) const CREDENTIALS: [&str; 1] = [".credentials.json"];
/// Empty writable directories Claude fills during a Session. They are created,
/// never wiped, so resume keeps the transcripts under `projects/`.
pub(super) const WRITABLE_DIRS: [&str; 3] = ["projects", "todos", "shell-snapshots"];
/// The config file Claude reads from inside `$CLAUDE_CONFIG_DIR`.
const CONFIG: &str = ".claude.json";
const STAGED: &str = ".orch-new";
const RETIRED: &str = ".orch-old";

static BUILDING: Mutex<()> = Mutex::new(());

pub(super) struct Dirs {
    pub(super) home: PathBuf,
    /// The real `~/.claude` (or `$CLAUDE_CONFIG_DIR`) the credentials and the
    /// carve-outs are linked from.
    pub(super) user: PathBuf,
    /// The synthesized config dir orch points `CLAUDE_CONFIG_DIR` at.
    pub(super) profile: PathBuf,
}

impl Dirs {
    pub(super) fn resolve(lookup: &dyn Fn(&str) -> Option<String>) -> Option<Self> {
        let set = |key: &str| lookup(key).filter(|value| !value.is_empty());
        let home = PathBuf::from(set("HOME")?);
        let state = set("XDG_STATE_HOME").map_or_else(|| home.join(".local/state"), PathBuf::from);
        let user = super::settings::config_dir(lookup)?;
        Some(Self {
            user,
            profile: state.join("orchestrator/agent-profiles/claude"),
            home,
        })
    }
}

/// Builds the minimal config dir orch exposes to the Sandbox: the credentials
/// and the read-only carve-outs are linked from the real `~/.claude`, the
/// writable state dirs are created empty, and a minimal `.claude.json` is
/// seeded. The host's `projects`, history, todos, shell-snapshots, statsig and
/// the real `.claude.json` never enter the profile.
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

    // The config carve-outs are always linked, even when the source is still
    // missing: the Sandbox resolves the link target and mounts it read-only (so
    // Claude reads the real file through the profile and can't plant into it),
    // creating an empty placeholder when the target does not exist yet. A
    // dangling link is never bound directly, so bwrap never sees a missing bind.
    for name in CONFIG_CARVE_OUTS {
        link(profile, name, &user.join(name))?;
    }
    // Credentials are linked only when they exist; a missing link would be a
    // writable bind with no source, which bwrap rejects.
    for name in CREDENTIALS {
        let target = user.join(name);
        if target.symlink_metadata().is_ok() {
            link(profile, name, &target)?;
        } else {
            remove(&profile.join(name))?;
        }
    }
    for dir in WRITABLE_DIRS {
        std::fs::create_dir_all(profile.join(dir))?;
    }
    write_config(profile, None)
}

/// Renders `.claude.json` into the profile, merging the existing one so a
/// parallel launch keeps its project. Marks onboarding complete and, when a
/// Session cwd is known, pre-trusts it so Claude honours `permissions.allow`.
pub(super) fn render_config(profile: &Path, cwd: Option<&Path>) -> io::Result<()> {
    let _building = BUILDING.lock().unwrap_or_else(PoisonError::into_inner);
    write_config(profile, cwd)
}

/// Merges and writes `.claude.json`. The caller holds `BUILDING`.
fn write_config(profile: &Path, cwd: Option<&Path>) -> io::Result<()> {
    let path = profile.join(CONFIG);
    let mut config: Map<String, Value> = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Map::new(),
        Err(err) => return Err(err),
    };
    config.insert("hasCompletedOnboarding".into(), json!(true));
    if let Some(cwd) = cwd {
        let projects = config
            .entry("projects")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(|| io::Error::other("projects is not an object"))?;
        let project = projects
            .entry(path_text(cwd))
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(|| io::Error::other("project is not an object"))?;
        project.insert("hasTrustDialogAccepted".into(), json!(true));
    }
    write_file(profile, CONFIG, &serde_json::to_vec_pretty(&config)?)
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
