use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use super::profile::path_text;
use crate::lines::open;
use crate::{Argv, DraftSpec};

const SESSIONS: &str = "sessions";
const FORK: &str = "conversation.jsonl";
const TOOLS: &str = "read,grep,find,ls";

pub(super) struct Fork {
    source: PathBuf,
    sessions: PathBuf,
}

pub(super) fn argv(program: &str, fork: &Fork) -> Argv {
    let source = path_text(&fork.source);
    let sessions = path_text(&fork.sessions);
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
            &source,
            "--session-dir",
            &sessions,
        ]
        .map(String::from)
        .into(),
    }
}

pub(super) fn copy_conversation(profile: &Path, spec: &DraftSpec) -> io::Result<Option<Fork>> {
    let Some(scratch) = &spec.scratch else {
        return Ok(None);
    };
    let Ok(sessions) = std::fs::canonicalize(profile).map(|profile| profile.join(SESSIONS)) else {
        return Ok(None);
    };
    let suffix = format!("_{}.jsonl", spec.conversation.as_str());
    let canonical = std::fs::canonicalize(&spec.worktree).ok();
    let Some(live) = std::iter::once(spec.worktree.as_path())
        .chain(canonical.as_deref())
        .find_map(|cwd| newest(&sessions.join(session_dir_name(cwd)), &suffix))
    else {
        return Ok(None);
    };
    let Some(mut file) = open(&live, Some(&sessions)) else {
        return Ok(None);
    };
    let fork = Fork {
        source: scratch.join(FORK),
        sessions: scratch.join(SESSIONS),
    };
    let mut copy = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&fork.source)?;
    io::copy(&mut file, &mut copy)?;
    Ok(Some(fork))
}

pub fn session_dir_name(cwd: &Path) -> String {
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
