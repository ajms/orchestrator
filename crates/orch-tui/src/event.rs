use std::fmt;
use std::path::PathBuf;
use std::process::ExitStatus;

use crossterm::event::Event as TermEvent;
use orch_core::SessionId;
use orch_protocol::FromDaemon;

use crate::review::FileDiff;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaneId(pub u64);

#[derive(Debug)]
pub enum Event {
    Daemon(FromDaemon),
    Pane {
        pane: PaneId,
        message: FromDaemon,
    },
    Terminal(TermEvent),
    EditorClosed(Result<String, EditorError>),
    Review {
        session: SessionId,
        purpose: ReviewPurpose,
        result: Result<ReviewData, orch_git::Error>,
    },
    VersionMismatch {
        message: String,
    },
    Disconnected {
        reason: String,
    },
    Notice(String),
    Tick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewPurpose {
    BuiltIn,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewTarget {
    pub repo: PathBuf,
    pub worktree: PathBuf,
    pub slug: String,
    pub branch: String,
    pub base: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewData {
    pub merge_base: String,
    pub tree: String,
    pub files: Vec<FileDiff>,
}

#[derive(Debug)]
pub enum EditorError {
    Io(std::io::Error),
    Exited { editor: String, status: ExitStatus },
}

impl fmt::Display for EditorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => err.fmt(f),
            Self::Exited { editor, status } => write!(f, "{editor} exited with {status}"),
        }
    }
}

impl std::error::Error for EditorError {}

impl From<std::io::Error> for EditorError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Quit,
    RestartDaemon,
    WriteTerminal(Vec<u8>),
    CopyCommand {
        program: String,
        args: Vec<String>,
        text: String,
    },
    EditText {
        text: String,
    },
    LoadReview {
        session: SessionId,
        target: ReviewTarget,
        purpose: ReviewPurpose,
    },
    RunExternal {
        command: String,
        cwd: PathBuf,
        env: Vec<(String, String)>,
    },
    OpenUrl {
        url: String,
    },
    OpenInEditor {
        file: PathBuf,
        line: Option<u32>,
        cwd: PathBuf,
    },
}
