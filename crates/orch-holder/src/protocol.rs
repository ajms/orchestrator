use orch_agent::GuardAnswer;
use orch_config::PortBlock;
use orch_core::SessionId;
use serde::{Deserialize, Serialize};

use crate::ScreenSnapshot;

pub const PROTOCOL_VERSION: u32 = 1;

pub type GuardId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub rows: u16,
    pub cols: u16,
}

impl Size {
    pub const DEFAULT: Size = Size { rows: 24, cols: 80 };
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToHolder {
    Attach {
        version: u32,
    },
    Ack {
        through: u64,
    },
    Snapshot,
    Subscribe,
    Input {
        #[serde(with = "crate::bytes")]
        bytes: Vec<u8>,
    },
    Paste {
        text: String,
    },
    Resize(Size),
    Kill,
    GuardHeld {
        id: GuardId,
    },
    GuardAnswer {
        id: GuardId,
        answer: GuardAnswer,
    },
    Shutdown,
    Hook {
        agent: String,
        payload: String,
    },
    Guard {
        agent: String,
        payload: String,
    },
    Tap {
        agent: String,
        payload: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FromHolder {
    Hello(Hello),
    Event {
        seq: u64,
        event: HolderEvent,
    },
    Screen(ScreenSnapshot),
    Output {
        #[serde(with = "crate::bytes")]
        bytes: Vec<u8>,
    },
    Resized(Size),
    GuardAnswer {
        answer: GuardAnswer,
    },
    Superseded,
    Clipboard {
        text: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub version: u32,
    pub session: SessionId,
    #[serde(default)]
    pub cwd: Option<std::path::PathBuf>,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub agent_name: Option<String>,
    #[serde(default)]
    pub port_block: Option<PortBlock>,
    pub holder_pid: u32,
    pub agent_pid: Option<u32>,
    pub agent: AgentStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AgentStatus {
    Running,
    Exited(AgentExit),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentExit {
    pub code: u32,
    pub signal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HolderEvent {
    Spawned {
        pid: Option<u32>,
    },
    Hook {
        #[serde(default)]
        agent: Option<String>,
        payload: String,
        guard: Option<GuardId>,
    },
    Tap {
        #[serde(default)]
        agent: Option<String>,
        payload: String,
    },
    Exited(AgentExit),
}

impl HolderEvent {
    pub fn agent(&self) -> Option<&str> {
        match self {
            Self::Hook { agent, .. } | Self::Tap { agent, .. } => agent.as_deref(),
            _ => None,
        }
    }
}
