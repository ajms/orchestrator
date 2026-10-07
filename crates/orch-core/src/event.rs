#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidSessionId(pub String);

impl SessionId {
    const MAX_LEN: usize = 64;

    pub fn parse(id: &str) -> Result<Self, InvalidSessionId> {
        let safe = id.len() <= Self::MAX_LEN
            && !id.starts_with('.')
            && !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if safe {
            Ok(Self(id.into()))
        } else {
            Err(InvalidSessionId(id.into()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConversationId(pub String);

impl ConversationId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubagentId(pub String);

impl SubagentId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PermissionMode {
    Default,
    AcceptEdits,
    Plan,
    Auto,
    DontAsk,
    BypassPermissions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureKind {
    RateLimited,
    Authentication,
    Billing,
    InvalidRequest,
    Server,
    MaxOutputTokens,
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageWindow {
    pub name: String,
    pub label: String,
    pub used_percent: f64,
    pub resets_at_unix: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageSample {
    pub conversation: Option<ConversationId>,
    pub model: Option<String>,
    pub context_used_percent: Option<f64>,
    pub context_window_tokens: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub windows: Vec<UsageWindow>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    SessionStarted,
    PromptSubmitted,
    ToolStarted {
        tool: String,
        subagent: Option<SubagentId>,
    },
    ToolFinished {
        tool: String,
        subagent: Option<SubagentId>,
    },
    PermissionRequested,
    PermissionDenied,
    QuestionAsked,
    TurnEnded,
    Failed {
        kind: FailureKind,
    },
    ModeChanged {
        mode: PermissionMode,
    },
    ConversationChanged {
        id: ConversationId,
    },
    TitleChanged {
        title: String,
    },
    UsageSample(UsageSample),
    SubagentStarted {
        id: SubagentId,
        agent_type: String,
        description: String,
    },
    SubagentFinished {
        id: SubagentId,
    },
    GuardCheck {
        tool: String,
        action: GuardedAction,
        cwd: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardedAction {
    WriteFile { path: String },
    Shell { command: String },
    ExternalTool { name: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Observation {
    Spawned,
    Agent(AgentEvent),
    Exited { code: Option<i32> },
    UserInput,
    GuardPrompted,
}
