use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Args, Subcommand};
use orch_config::ConfigLoader;
use orch_daemon::{DaemonConfig, NotificationTarget};
use orch_holder::default_runtime_dir;
use orch_store::Store;

use crate::client::fail;

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct DaemonArgs {
    #[command(subcommand)]
    action: Option<DaemonAction>,
    #[arg(long, conflicts_with = "idle_timeout_ms")]
    no_idle_exit: bool,
    #[arg(long)]
    idle_timeout_ms: Option<u64>,
    #[arg(long, hide = true, default_value_t = DaemonConfig::DEFAULT_PR_POLL.as_millis() as u64)]
    pr_poll_ms: u64,
    #[arg(long, hide = true, default_value_t = DaemonConfig::DEFAULT_RECONCILE.as_millis() as u64)]
    reconcile_ms: u64,
    #[arg(long, hide = true)]
    notify_log: Option<PathBuf>,
}

#[derive(Subcommand)]
enum DaemonAction {
    Install,
    Uninstall,
}

pub fn run(args: DaemonArgs) -> ExitCode {
    match args.action {
        Some(DaemonAction::Install) => return crate::systemd::install(),
        Some(DaemonAction::Uninstall) => return crate::systemd::uninstall(),
        None => {}
    }
    let (Some(loader), Some(state_db)) = (ConfigLoader::from_env(), Store::default_path()) else {
        return fail("daemon", 1, "neither XDG base directories nor HOME are set");
    };
    let orch_program = match std::env::current_exe() {
        Ok(program) => program,
        Err(err) => {
            return fail("daemon", 1, err);
        }
    };
    let sessions_dir = state_db.with_file_name("sessions");
    let config = DaemonConfig {
        runtime_dir: default_runtime_dir(),
        state_db,
        sessions_dir,
        loader,
        orch_program,
        idle_timeout: (!args.no_idle_exit).then(|| {
            args.idle_timeout_ms
                .map_or(DaemonConfig::DEFAULT_IDLE_TIMEOUT, Duration::from_millis)
        }),
        pr_poll_interval: Duration::from_millis(args.pr_poll_ms),
        reconcile_interval: Duration::from_millis(args.reconcile_ms),
        notifications: args
            .notify_log
            .map_or(NotificationTarget::Desktop, NotificationTarget::Log),
    };
    match orch_daemon::run(config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => fail("daemon", 1, err),
    }
}
