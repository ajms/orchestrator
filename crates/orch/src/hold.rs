use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::Args;
use orch_config::PortBlock;
use orch_core::SessionId;
use orch_holder::{HoldConfig, Size, default_runtime_dir, socket_path};

#[derive(Args)]
pub struct HoldArgs {
    #[arg(long, value_parser = parse_session)]
    session: SessionId,
    #[arg(long)]
    runtime_dir: Option<PathBuf>,
    #[arg(long)]
    cwd: Option<PathBuf>,
    #[arg(long = "env", value_parser = parse_env)]
    env: Vec<(String, String)>,
    #[arg(long)]
    base: Option<String>,
    #[arg(long, requires = "port_size")]
    port_base: Option<u16>,
    #[arg(long, requires = "port_base")]
    port_size: Option<u16>,
    #[arg(long, default_value_t = Size::DEFAULT.rows)]
    rows: u16,
    #[arg(long, default_value_t = Size::DEFAULT.cols)]
    cols: u16,
    #[arg(long, default_value_t = HoldConfig::DEFAULT_GUARD_TIMEOUT.as_millis() as u64)]
    guard_timeout_ms: u64,
    #[arg(long, default_value_t = HoldConfig::DEFAULT_EVENT_CAPACITY)]
    event_capacity: usize,
    #[arg(long)]
    foreground: bool,
    #[arg(last = true, required = true)]
    argv: Vec<String>,
}

fn parse_session(id: &str) -> Result<SessionId, String> {
    SessionId::parse(id).map_err(|_| format!("invalid session id {id:?}"))
}

fn parse_env(pair: &str) -> Result<(String, String), String> {
    pair.split_once('=')
        .map(|(key, value)| (key.into(), value.into()))
        .ok_or_else(|| format!("expected KEY=VALUE, got {pair}"))
}

pub fn run(args: HoldArgs) -> ExitCode {
    let runtime_dir = args.runtime_dir.unwrap_or_else(default_runtime_dir);
    let cwd = match args.cwd.map_or_else(std::env::current_dir, Ok) {
        Ok(cwd) => cwd,
        Err(err) => {
            eprintln!("orch hold: {err}");
            return ExitCode::FAILURE;
        }
    };
    let socket = socket_path(&runtime_dir, &args.session);
    let mut config = HoldConfig::new(args.session, socket, cwd, args.argv);
    config.env = args.env;
    config.base = args.base;
    config.port_block = args
        .port_base
        .zip(args.port_size)
        .map(|(base, size)| PortBlock { base, size });
    config.size = Size {
        rows: args.rows,
        cols: args.cols,
    };
    config.guard_timeout = Duration::from_millis(args.guard_timeout_ms);
    config.event_capacity = args.event_capacity;
    let result = if args.foreground {
        orch_holder::hold(config)
    } else {
        orch_holder::hold_detached(config).map(|pid| println!("{pid}"))
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("orch hold: {err}");
            ExitCode::FAILURE
        }
    }
}
