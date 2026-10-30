mod broker;
mod client;
mod daemon;
mod doctor;
mod fake_agent;
mod hold;
mod hook;
mod repo;
mod subprocess;
mod systemd;
mod tap;
mod trust;
mod tui;
mod worktree_hook;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use orch_agent::CLAUDE;

const VERSION: &str = match option_env!("ORCH_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};

#[derive(Parser)]
#[command(name = "orch", version = VERSION)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    #[command(subcommand, hide = true)]
    Broker(broker::BrokerCommand),
    Daemon(daemon::DaemonArgs),
    Doctor(doctor::DoctorArgs),
    Hold(hold::HoldArgs),
    Hook {
        #[arg(long)]
        session: String,
        #[arg(long, default_value = CLAUDE)]
        agent: String,
    },
    #[command(subcommand)]
    Repo(repo::RepoCommand),
    Tap {
        #[arg(long)]
        session: String,
        #[arg(long, default_value = CLAUDE)]
        agent: String,
    },
    Trust(trust::TrustArgs),
    #[command(hide = true)]
    WorktreeHook {
        #[arg(long)]
        worktree: PathBuf,
        #[arg(long, default_value = CLAUDE)]
        agent: String,
    },
    #[command(hide = true)]
    FakeAgent {
        #[arg(long)]
        script: Option<PathBuf>,
        #[arg(last = true)]
        agent_args: Vec<String>,
    },
}

fn main() -> ExitCode {
    match Cli::parse().command {
        None => tui::run(),
        Some(Command::Broker(command)) => broker::run(command),
        Some(Command::Daemon(args)) => daemon::run(args),
        Some(Command::Doctor(args)) => doctor::run(args),
        Some(Command::Hold(args)) => hold::run(args),
        Some(Command::Hook { session, agent }) => hook::run(&session, &agent),
        Some(Command::Repo(command)) => repo::run(command),
        Some(Command::Tap { session, agent }) => tap::run(&session, &agent),
        Some(Command::Trust(args)) => trust::run(args),
        Some(Command::WorktreeHook { worktree, agent }) => worktree_hook::run(&worktree, &agent),
        Some(Command::FakeAgent { script, agent_args }) => {
            fake_agent::run(script.as_deref(), &agent_args)
        }
    }
}
