use std::path::PathBuf;

use orch_core::SessionId;
use orch_holder::{ScreenSnapshot, Size};
use serde::{Deserialize, Serialize};

use crate::view::SessionView;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToDaemon {
    Hello {
        version: u32,
        #[serde(default)]
        pane: Option<OpenPane>,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum Reply {
    Created { session: SessionId },
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum RequestError {
    Untrusted { hash: String, items: Vec<String> },
    UnknownSession,
    Refused { message: String },
}

impl std::fmt::Display for RequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Untrusted { items, .. } => {
                write!(f, "the Repo's config needs Trust for: {}", items.join(", "))
            }
            Self::UnknownSession => f.write_str("no such Session"),
            Self::Refused { message } => f.write_str(message),
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
