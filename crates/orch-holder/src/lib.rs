mod bytes;
mod client;
mod connection;
mod detach;
mod event_log;
mod frame;
mod guards;
mod holder;
mod paths;
mod protocol;
mod report;
mod screen;

pub use client::{HolderClient, HolderReader};
pub use detach::hold_detached;
pub use frame::{read_frame, read_frame_async, write_frame, write_frame_async};
pub use holder::{HoldConfig, hold};
pub use paths::{
    HOLDER_SOCKET_ENV, RUNTIME_DIR_ENV, SESSION_ENV, default_runtime_dir, locate_socket,
    socket_path,
};
pub use protocol::{
    AgentExit, AgentStatus, FromHolder, GuardId, Hello, HolderEvent, PROTOCOL_VERSION, Size,
    ToHolder,
};
pub use report::{report, request_guard};
pub use screen::{Emulator, ScreenCopy, ScreenSnapshot};
