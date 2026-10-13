use std::fmt;
use std::io;
use std::path::PathBuf;

use crate::PortRange;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigProblem {
    UnknownPermissionMode { preset: String, mode: String },
    ReservedPresetName,
    EmptyPortRange(PortRange),
    Misplaced { key: &'static str },
}

impl fmt::Display for ConfigProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigProblem::UnknownPermissionMode { preset, mode } => {
                write!(f, "preset {preset}: unknown permission mode {mode:?}")
            }
            ConfigProblem::ReservedPresetName => {
                write!(f, "preset name {:?} is reserved", orch_agent::INHERIT)
            }
            ConfigProblem::EmptyPortRange(range) => write!(
                f,
                "port range {}..={} has no room for a block of {}",
                range.start, range.end, range.block_size
            ),
            ConfigProblem::Misplaced { key } => write!(f, "{key} is not allowed here"),
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io {
        path: PathBuf,
        source: io::Error,
    },
    Parse {
        path: PathBuf,
        message: String,
    },
    Invalid {
        path: PathBuf,
        problem: ConfigProblem,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io { path, source } => write!(f, "{}: {source}", path.display()),
            ConfigError::Parse { path, message } => write!(f, "{}: {message}", path.display()),
            ConfigError::Invalid { path, problem } => write!(f, "{}: {problem}", path.display()),
        }
    }
}

impl std::error::Error for ConfigError {}
