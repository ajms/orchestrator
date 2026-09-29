mod client;
mod message;
mod view;

pub use client::{
    Client, ConnectError, DEFAULT_SPAWN_WAIT, Pane, connect_or_spawn, connect_or_spawn_daemon,
    daemon_command, daemon_lock, daemon_socket, restart_daemon, restart_running_daemon,
    spawn_detached, stop_daemon,
};
pub use message::{
    CreateSession, FromDaemon, GuardChoice, OpenPane, PROTOCOL_VERSION, Reply, Request,
    RequestError, ToDaemon,
};
pub use orch_holder::{ScreenSnapshot, Size};
pub use view::{
    AgentStateView, FlagsView, GuardKindView, GuardPrompt, PhaseView, PrChecksView, PrReviewView,
    PrView, SessionView, SubagentView,
};
