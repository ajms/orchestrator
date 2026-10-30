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
        payload: String,
        #[serde(default = "orch_agent::implied_agent")]
        agent: String,
    },
    Guard {
        payload: String,
        #[serde(default = "orch_agent::implied_agent")]
        agent: String,
    },
    Tap {
        payload: String,
        #[serde(default = "orch_agent::implied_agent")]
        agent: String,
    },
    Broker {
        request: BrokerRequest,
    },
    BrokerReply {
        id: GuardId,
        reply: BrokerReply,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrokerRequest {
    Refs,
    OriginRefs,
    Fetch { wants: Vec<RemoteRef> },
    Push { updates: Vec<RefUpdate> },
    Gh { args: Vec<String> },
    Sign { payload: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefUpdate {
    pub src: String,
    pub commit: Option<String>,
    pub dst: String,
    pub force: bool,
    pub lease: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrokerReply {
    Refs { refs: Vec<RemoteRef> },
    Done { said: String },
    Refused { reason: String },
    Gh { said: String, output: GhOutput },
    Signed { signature: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GhOutput {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteRef {
    pub name: String,
    pub commit: String,
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
    BrokerReply {
        reply: BrokerReply,
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
    pub port_block: Option<PortBlock>,
    pub holder_pid: u32,
    pub agent_pid: Option<u32>,
    pub agent: AgentStatus,
    #[serde(default)]
    pub sandboxed: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_failure: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HolderEvent {
    Spawned {
        pid: Option<u32>,
    },
    Hook {
        payload: String,
        guard: Option<GuardId>,
        #[serde(default = "orch_agent::implied_agent")]
        agent: String,
    },
    Tap {
        payload: String,
        #[serde(default = "orch_agent::implied_agent")]
        agent: String,
    },
    Broker {
        id: GuardId,
        request: BrokerRequest,
    },
    Exited(AgentExit),
}
