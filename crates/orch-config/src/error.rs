use std::fmt;
use std::io;
use std::path::PathBuf;

use crate::PortRange;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigProblem {
    UnknownPermissionMode {
        preset: String,
        mode: String,
    },
    ReservedPresetName,
    ClaudeRuleTable {
        preset: String,
    },
    UnknownRuleAgent {
        preset: String,
        agent: String,
    },
    EmptyPortRange(PortRange),
    Misplaced {
        key: &'static str,
    },
    OldAgentTable {
        name: String,
        binary: Option<String>,
        args: Option<Vec<String>>,
    },
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
            ConfigProblem::ClaudeRuleTable { preset } => write!(
                f,
                "preset {preset}: write Claude rules as allow/deny in [presets.{preset}], not in [presets.{preset}.claude]"
            ),
            ConfigProblem::UnknownRuleAgent { preset, agent } => write!(
                f,
                "preset {preset}: [presets.{preset}.{agent}] names no known Agent ({})",
                orch_agent::built_in_names().collect::<Vec<_>>().join(", ")
            ),
            ConfigProblem::EmptyPortRange(range) => write!(
                f,
                "port range {}..={} has no room for a block of {}",
                range.start, range.end, range.block_size
            ),
            ConfigProblem::Misplaced { key } => write!(f, "{key} is not allowed here"),
            ConfigProblem::OldAgentTable { name, binary, args } => {
                write!(f, "the [agent] table was replaced; write agent = {name:?}")?;
                if binary.is_some() || args.is_some() {
                    write!(f, " and [agents.{name}]")?;
                    if let Some(binary) = binary {
                        write!(f, " binary = {binary:?}")?;
                    }
                    if let Some(args) = args {
                        write!(f, " args = {args:?}")?;
                    }
                }
                Ok(())
            }
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
