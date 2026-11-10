use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::process::ExitStatus;

use crossterm::event::Event as TermEvent;
use orch_core::SessionId;
use orch_protocol::{FromDaemon, Size};

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
    Shell {
        pane: PaneId,
        event: ShellEvent,
    },
    EditorClosed(Result<String, EditorError>),
    Review {
        session: SessionId,
        scope: ReviewScope,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellEvent {
    Output(Vec<u8>),
    Exited(ShellExit),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellExit {
    Code(u32),
    Signal(String),
    Failed(String),
}

impl fmt::Display for ShellExit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Code(code) => write!(f, "exit status: {code}"),
            Self::Signal(signal) => write!(f, "killed by {signal}"),
            Self::Failed(err) => write!(f, "failed to start: {err}"),
        }
    }
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
    pub scope: ReviewScope,
}

/// What a Review covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewScope {
    /// The Session's work against its base.
    Session,
    /// Only this Subagent's work: its recorded base against its tip (a live
    /// snapshot, or the tip recorded when its Subagent worktree was removed).
    Subagent(SubagentName),
}

/// The name of a Subagent worktree: a single path component, so it names a
/// directory directly inside the Session's Subagent worktrees dir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentName(String);

impl SubagentName {
    pub fn new(name: &str) -> Option<Self> {
        let single = matches!(
            Path::new(name).components().collect::<Vec<_>>()[..],
            [Component::Normal(_)]
        );
        single.then(|| Self(name.to_owned()))
    }

    /// The name of the Subagent worktree at `path`, if it is directly inside
    /// the Subagent worktrees dir of the Session at `session_worktree`.
    pub(crate) fn of_worktree(session_worktree: &Path, path: &Path) -> Option<Self> {
        let inside = path.parent()
            == Some(orch_git::subagent_worktrees_dir(session_worktree).as_path())
            && !path.components().any(|part| part == Component::ParentDir);
        inside
            .then(|| path.file_name()?.to_str())
            .flatten()
            .and_then(Self::new)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Where this Subagent worktree lives for the Session at `session_worktree`.
    pub fn worktree(&self, session_worktree: &Path) -> PathBuf {
        orch_git::subagent_worktrees_dir(session_worktree).join(&self.0)
    }

    /// The branch this Subagent worktree was created on.
    pub fn branch(&self) -> String {
        orch_git::subagent_branch(&self.0)
    }
}

impl fmt::Display for SubagentName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
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
    OpenShell {
        pane: PaneId,
        command: Option<String>,
        cwd: PathBuf,
        size: Size,
    },
    ShellInput {
        pane: PaneId,
        bytes: Vec<u8>,
    },
    ResizeShell {
        pane: PaneId,
        size: Size,
    },
    CloseShell {
        pane: PaneId,
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
