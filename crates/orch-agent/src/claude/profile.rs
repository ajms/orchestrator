use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use nix::libc::O_NOFOLLOW;
use orch_core::ConversationId;
use serde_json::{Map, Value, json};

use super::CONFIG_CARVE_OUTS;
use crate::profile::{self, create, link, open_root, path_text, remove, staged, swap, write_file};

pub(super) const CREDENTIALS: [&str; 1] = [".credentials.json"];
pub(super) const WRITABLE_DIRS: [&str; 4] = ["projects", "todos", "shell-snapshots", "plans"];
pub(super) const CONFIG: &str = ".claude.json";
const TRUSTED: &str = "hasTrustDialogAccepted";
const MCP_SERVERS: &str = "mcpServers";

static BUILDING: Mutex<()> = Mutex::new(());

pub(super) fn dirs(lookup: &dyn Fn(&str) -> Option<String>) -> Option<profile::Dirs> {
    profile::Dirs::resolve(lookup, "claude", |_| super::settings::config_dir(lookup))
}

pub(super) fn user_config(lookup: &dyn Fn(&str) -> Option<String>, home: &Path) -> PathBuf {
    lookup("CLAUDE_CONFIG_DIR")
        .filter(|dir| !dir.is_empty())
        .map_or_else(|| home.to_path_buf(), PathBuf::from)
        .join(CONFIG)
}

/// Builds the Agent profile a Claude run on the host reads: links to the
/// user's carve-outs and credentials, the state dirs and `.claude.json`.
pub(super) fn build(user: &Path, user_config: &Path, profile: &Path) -> io::Result<()> {
    let _building = BUILDING.lock().unwrap_or_else(PoisonError::into_inner);
    open_root(profile)?;
    for name in CONFIG_CARVE_OUTS.into_iter().chain(CREDENTIALS) {
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
    write_config(user_config, profile, None)
}

pub(super) fn render_config(user_config: &Path, profile: &Path, cwd: &Path) -> io::Result<()> {
    let _building = BUILDING.lock().unwrap_or_else(PoisonError::into_inner);
    write_config(user_config, profile, Some(cwd))
}

/// Copies a Conversation from before the Agent profile into it, once.
pub(super) fn adopt_conversation(
    user: &Path,
    profile: &Path,
    cwd: &Path,
    conversation: &ConversationId,
) -> io::Result<()> {
    let name = conversation.as_str();
    if Path::new(name).file_name().and_then(|file| file.to_str()) != Some(name) {
        return Ok(());
    }
    let slug = project_slug(cwd);
    let from = user.join("projects").join(&slug);
    let to = profile.join("projects").join(&slug);
    let file = format!("{name}.jsonl");
    let _building = BUILDING.lock().unwrap_or_else(PoisonError::into_inner);
    let source = from.join(&file);
    if !source.is_file() || to.join(&file).symlink_metadata().is_ok() {
        return Ok(());
    }
    if !to.symlink_metadata().is_ok_and(|meta| meta.is_dir()) {
        remove(&to)?;
        std::fs::create_dir(&to)?;
    }
    if from.join(name).is_dir() {
        let staged = staged(&to, name)?;
        copy_tree(&from.join(name), &staged)?;
        swap(&to, name, &staged)?;
    }
    write_file(&to, &file, &std::fs::read(source)?)
}

fn project_slug(cwd: &Path) -> String {
    path_text(cwd)
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            io::copy(
                &mut std::fs::File::open(entry.path())?,
                &mut create(&target)?,
            )?;
        }
    }
    Ok(())
}

fn write_config(user_config: &Path, profile: &Path, cwd: Option<&Path>) -> io::Result<()> {
    let mut projects = trusted_projects(&profile.join(CONFIG))?;
    if let Some(cwd) = cwd {
        projects.insert(path_text(cwd), json!({ TRUSTED: true }));
    }
    let mut config = json!({ "hasCompletedOnboarding": true, "projects": projects });
    if let Some(servers) = user_mcp_servers(user_config) {
        config[MCP_SERVERS] = servers;
    }
    write_file(profile, CONFIG, &serde_json::to_vec_pretty(&config)?)
}

fn user_mcp_servers(user_config: &Path) -> Option<Value> {
    let bytes = std::fs::read(user_config).ok()?;
    let mut config = serde_json::from_slice::<Value>(&bytes).ok()?;
    Some(config.get_mut(MCP_SERVERS)?.take()).filter(Value::is_object)
}

fn trusted_projects(path: &Path) -> io::Result<Map<String, Value>> {
    let Some(bytes) = read_unlinked(path)? else {
        return Ok(Map::new());
    };
    let config = serde_json::from_slice::<Value>(&bytes).unwrap_or_default();
    let trusted = config["projects"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, project)| project[TRUSTED] == true)
        .map(|(cwd, _)| (cwd.clone(), json!({ TRUSTED: true })))
        .collect();
    Ok(trusted)
}

fn read_unlinked(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let opened = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW)
        .open(path);
    let mut file = match opened {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) if err.raw_os_error() == Some(nix::libc::ELOOP) => return Ok(None),
        Err(err) => return Err(err),
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}
