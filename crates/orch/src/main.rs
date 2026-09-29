mod fake_agent;
mod hold;
mod hook;
mod subprocess;
mod tap;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "orch", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Hold(hold::HoldArgs),
    Hook {
        #[arg(long)]
        session: String,
    },
    Tap {
        #[arg(long)]
        session: String,
    },
    #[command(hide = true)]
    FakeAgent {
        #[arg(long)]
        script: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    match Cli::parse().command {
        None => {
            println!("orch {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some(Command::Hold(args)) => hold::run(args),
        Some(Command::Hook { session }) => hook::run(&session),
        Some(Command::Tap { session }) => tap::run(&session),
        Some(Command::FakeAgent { script }) => fake_agent::run(script.as_deref()),
    }
}
