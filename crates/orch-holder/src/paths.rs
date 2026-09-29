use std::path::{Path, PathBuf};

use orch_config::xdg;
use orch_core::SessionId;

pub const HOLDER_SOCKET_ENV: &str = "ORCH_HOLDER_SOCKET";
pub const SESSION_ENV: &str = "ORCH_SESSION";
pub const RUNTIME_DIR_ENV: &str = "ORCH_RUNTIME_DIR";

pub fn default_runtime_dir() -> PathBuf {
    if let Some(dir) = xdg::process_env(RUNTIME_DIR_ENV).filter(|dir| !dir.is_empty()) {
        return dir.into();
    }
    match xdg::runtime_dir(xdg::process_env) {
        Some(dir) => dir.join("orchestrator"),
        None => std::env::temp_dir().join(format!("orchestrator-{}", nix::unistd::getuid())),
    }
}

pub fn holders_dir(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("holders")
}

pub fn socket_path(runtime_dir: &Path, session: &SessionId) -> PathBuf {
    holders_dir(runtime_dir).join(format!("{}.sock", session.as_str()))
}

pub fn locate_socket(session: &SessionId) -> PathBuf {
    xdg::process_env(HOLDER_SOCKET_ENV)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| socket_path(&default_runtime_dir(), session))
}
