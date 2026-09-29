use std::path::PathBuf;

use orch_core::SessionId;
use orch_holder::{ScreenSnapshot, Size};
use serde::{Deserialize, Serialize};

use crate::reconcile::{Fix, LeftoverView, ReconcileReport};
use crate::view::SessionView;

pub const PROTOCOL_VERSION: u32 = 7;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToDaemon {
    Hello {
        version: u32,
        #[serde(default)]
        pane: Option<OpenPane>,
        #[serde(default)]
        display: Option<DisplayVars>,
    },
    Request {
        id: u64,
        request: Request,
    },
    Input {
        #[serde(with = "bytes")]
        bytes: Vec<u8>,
    },
    Paste {
        text: String,
    },
    Resize(Size),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayVars {
    pub wayland_display: Option<String>,
    pub x11_display: Option<String>,
}

impl DisplayVars {
    const WAYLAND: &str = "WAYLAND_DISPLAY";
    const X11: &str = "DISPLAY";

    pub fn from_env() -> Self {
        Self::from_vars(|key| std::env::var(key).ok())
    }

    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Self {
        let set = |key| var(key).filter(|value| !value.is_empty());
        Self {
            wayland_display: set(Self::WAYLAND),
            x11_display: set(Self::X11),
        }
    }

    pub(crate) fn reported(&self) -> Option<Self> {
        (self != &Self::default()).then(|| self.clone())
    }

    pub fn pairs(&self) -> Vec<(String, String)> {
        [
            (Self::WAYLAND, &self.wayland_display),
            (Self::X11, &self.x11_display),
        ]
        .into_iter()
        .filter_map(|(key, value)| Some((key.to_string(), value.clone()?)))
        .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenPane {
    pub session: SessionId,
    pub size: Size,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "snake_case")]
pub enum Request {
    CreateSession(CreateSession),
    ApproveTrust {
        repo: PathBuf,
        hash: String,
    },
    RetrySetup {
        session: SessionId,
    },
    StartAnyway {
        session: SessionId,
    },
    Resume {
        session: SessionId,
    },
    AnswerGuard {
        session: SessionId,
        guard: u64,
        choice: GuardChoice,
    },
    SetGuards {
        session: SessionId,
        enabled: bool,
    },
    View {
        session: Option<SessionId>,
        focused: bool,
    },
    Draft {
        session: SessionId,
        mode: LandingMode,
    },
    Land {
        session: SessionId,
        landing: Landing,
        #[serde(default)]
        skip_teardown: bool,
    },
    RefreshPr {
        session: SessionId,
    },
    AbandonPr {
        session: SessionId,
    },
    DiscardPreview {
        session: SessionId,
    },
    Discard {
        session: SessionId,
        #[serde(default)]
        skip_teardown: bool,
    },
    SetPreset {
        session: SessionId,
        preset: String,
    },
    SetMuted {
        session: SessionId,
        muted: bool,
    },
    Reconcile,
    Usage,
    LeftoverPreview {
        repo: PathBuf,
        leftover: LeftoverView,
    },
    Fix {
        fix: Fix,
    },
    Repos,
    RepoSettings {
        repo: PathBuf,
    },
    MoveRepo {
        from: PathBuf,
        to: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LandingMode {
    Squash,
    Pr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Landing {
    Squash { message: String },
    Pr { title: String, body: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitView {
    pub id: String,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateSession {
    pub repo: PathBuf,
    pub prompt: String,
    pub branch: Option<String>,
    pub base: Option<String>,
    pub preset: Option<String>,
}

impl CreateSession {
    pub fn new(repo: impl Into<PathBuf>, prompt: impl Into<String>) -> Self {
        Self {
            repo: repo.into(),
            prompt: prompt.into(),
            branch: None,
            base: None,
            preset: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardChoice {
    AllowOnce,
    AllowForSession,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FromDaemon {
    Welcome {
        version: u32,
    },
    VersionMismatch {
        daemon_version: u32,
        message: String,
    },
    Sessions {
        sessions: Vec<SessionView>,
    },
    SessionChanged {
        session: Box<SessionView>,
    },
    SessionRemoved {
        session: SessionId,
    },
    Reconciled {
        report: Box<ReconcileReport>,
    },
    Response {
        id: u64,
        result: Result<Reply, RequestError>,
    },
    Screen(ScreenSnapshot),
    Output {
        #[serde(with = "bytes")]
        bytes: Vec<u8>,
    },
    Resized(Size),
    PaneClosed {
        reason: String,
    },
    InputDropped {
        reason: String,
    },
    Ring {
        session: SessionId,
        title: String,
        body: String,
    },
    Focus {
        session: SessionId,
    },
    RateLimits {
        five_hour: Option<f64>,
        seven_day: Option<f64>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum Reply {
    Created {
        session: SessionId,
    },
    Done,
    Drafted {
        title: String,
        body: String,
    },
    Landed {
        commit: String,
        #[serde(default)]
        warning: Option<String>,
    },
    PrOpened {
        number: u64,
        #[serde(default)]
        committed_changes: bool,
    },
    DiscardPreview {
        uncommitted: Vec<String>,
        unlanded: Vec<CommitView>,
    },
    Reconciled {
        report: Box<ReconcileReport>,
    },
    Usage(UsageReport),
    Repos {
        repos: Vec<PathBuf>,
    },
    RepoSettings(RepoSettings),
    RepoMoved {
        repo: PathBuf,
        stale_overrides: StaleOverrides,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoSettings {
    pub repo: PathBuf,
    pub presets: Vec<String>,
    pub default_preset: Option<String>,
    pub default_base: Option<String>,
    pub review_command: Option<String>,
    pub branch_prefix: String,
    pub trust: Option<TrustNeeded>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustNeeded {
    pub hash: String,
    pub items: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaleOverrides {
    pub keys: Vec<String>,
    pub unreadable: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageReport {
    pub per_repo: Vec<RepoUsage>,
    pub today: Vec<RepoUsage>,
    pub total: UsageTotalsView,
    pub estimated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepoUsage {
    pub repo: PathBuf,
    pub totals: UsageTotalsView,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageTotalsView {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum RequestError {
    Untrusted {
        repo: PathBuf,
        hash: String,
        items: Vec<String>,
    },
    UnknownSession,
    Refused {
        message: String,
    },
    Conflict {
        paths: Vec<String>,
    },
    Internal {
        message: String,
    },
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Untrusted { items, .. } => {
                write!(f, "the Repo's config needs Trust for: {}", items.join(", "))
            }
            Self::UnknownSession => f.write_str("no such Session"),
            Self::Refused { message } => f.write_str(message),
            Self::Conflict { paths } => write!(f, "conflict in: {}", paths.join(", ")),
            Self::Internal { message } => write!(f, "internal error: {message}"),
        }
    }
}

impl std::error::Error for RequestError {}

mod bytes {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        STANDARD.decode(text).map_err(serde::de::Error::custom)
    }
}
