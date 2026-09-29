use std::process::ExitCode;
use std::time::Duration;

use clap::Args;
use orch_config::ConfigLoader;
use orch_daemon::DaemonConfig;
use orch_holder::default_runtime_dir;
use orch_store::Store;

#[derive(Args)]
pub struct DaemonArgs {
    #[arg(long, default_value_t = DaemonConfig::DEFAULT_IDLE_TIMEOUT.as_millis() as u64)]
    idle_timeout_ms: u64,
    #[arg(long, hide = true, default_value_t = DaemonConfig::DEFAULT_PR_POLL.as_millis() as u64)]
    pr_poll_ms: u64,
}

pub fn run(args: DaemonArgs) -> ExitCode {
    let (Some(loader), Some(state_db)) = (ConfigLoader::from_env(), Store::default_path()) else {
        eprintln!("orch daemon: neither XDG base directories nor HOME are set");
        return ExitCode::FAILURE;
    };
    let orch_program = match std::env::current_exe() {
        Ok(program) => program,
        Err(err) => {
            eprintln!("orch daemon: {err}");
            return ExitCode::FAILURE;
        }
    };
    let sessions_dir = state_db.with_file_name("sessions");
    let config = DaemonConfig {
        runtime_dir: default_runtime_dir(),
        state_db,
        sessions_dir,
        loader,
        orch_program,
        idle_timeout: Duration::from_millis(args.idle_timeout_ms),
        pr_poll_interval: Duration::from_millis(args.pr_poll_ms),
    };
    match orch_daemon::run(config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("orch daemon: {err}");
            ExitCode::FAILURE
        }
    }
}
