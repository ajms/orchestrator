#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConversationId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubagentId(pub String);

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RateLimit {
    pub used_percent: f64,
    pub resets_at_unix: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageSample {
    pub model: Option<String>,
    pub context_used_percent: Option<f64>,
    pub context_window_tokens: Option<u64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub five_hour: Option<RateLimit>,
    pub seven_day: Option<RateLimit>,
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
        input_json: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Observation {
    Spawned,
    Agent(AgentEvent),
    Exited { code: Option<i32> },
    UserInput,
    GuardPrompted,
}
