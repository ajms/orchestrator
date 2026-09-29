mod client;
mod message;
mod reconcile;
mod view;

pub use client::{
    Client, ConnectError, DEFAULT_SPAWN_WAIT, Pane, connect_or_spawn, connect_or_spawn_daemon,
    daemon_command, daemon_lock, daemon_socket, open_connection, restart_daemon,
    restart_running_daemon, spawn_detached, stop_daemon,
};
pub use message::{
    CommitView, CreateSession, FromDaemon, GuardChoice, Landing, LandingMode, OpenPane,
    PROTOCOL_VERSION, Reply, RepoUsage, Request, RequestError, ToDaemon, UsageReport,
    UsageTotalsView,
};
pub use orch_holder::{ScreenSnapshot, Size};
pub use reconcile::{Finding, Fix, LeftoverView, Problem, ReconcileReport, Repair, RepoReport};
pub use view::{
    AgentStateView, FlagsView, GuardKindView, GuardPrompt, PhaseView, PrChecksView, PrReviewView,
    PrView, SessionView, SubagentView,
};
