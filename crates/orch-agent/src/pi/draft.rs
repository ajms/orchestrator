use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use orch_core::ConversationId;

use super::profile::path_text;
use crate::Argv;
use crate::lines::open;

const SESSIONS: &str = "sessions";
const FORK: &str = "conversation.jsonl";
const TOOLS: &str = "read,grep,find,ls";

pub(super) fn argv(program: &str, scratch: &Path) -> Argv {
    let fork = path_text(&scratch.join(FORK));
    let sessions = path_text(&scratch.join(SESSIONS));
    Argv {
        program: program.into(),
        args: [
            "-p",
            "-ne",
            "-ns",
            "-np",
            "--tools",
            TOOLS,
            "--fork",
            &fork,
            "--session-dir",
            &sessions,
        ]
        .map(String::from)
        .into(),
    }
}

pub(super) fn copy_conversation(
    profile: &Path,
    worktree: &Path,
    conversation: &ConversationId,
    scratch: &Path,
) -> io::Result<bool> {
    let Ok(sessions) = std::fs::canonicalize(profile).map(|profile| profile.join(SESSIONS)) else {
        return Ok(false);
    };
    let suffix = format!("_{}.jsonl", conversation.as_str());
    let canonical = std::fs::canonicalize(worktree).ok();
    let Some(live) = std::iter::once(worktree)
        .chain(canonical.as_deref())
        .find_map(|cwd| newest(&sessions.join(cwd_dir(cwd)), &suffix))
    else {
        return Ok(false);
    };
    let Some(mut file) = open(&live, Some(&sessions)) else {
        return Ok(false);
    };
    let mut fork = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(scratch.join(FORK))?;
    io::copy(&mut file, &mut fork)?;
    Ok(true)
}

fn cwd_dir(cwd: &Path) -> String {
    let cwd = path_text(cwd);
    let cwd = cwd.strip_prefix(['/', '\\']).unwrap_or(&cwd);
    format!("--{}--", cwd.replace(['/', '\\', ':'], "-"))
}

fn newest(dir: &Path, suffix: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.ends_with(suffix))
        .max()
        .map(|name| dir.join(name))
}
