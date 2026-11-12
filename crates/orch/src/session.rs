use std::process::ExitCode;

use clap::Subcommand;
use orch_core::{SessionOp, TaskId};
use orch_holder::{BrokerReply, BrokerRequest};

use crate::agent_call;

/// Starts and finishes Child Sessions of the Session this runs in. Only the
/// Session's Agent reaches it, through its Holder socket, and it can act only
/// on its own Session and that Session's descendants.
#[derive(Subcommand)]
pub enum SessionCommand {
    /// Start a Child Session on this Session's Branch; prints its id at once.
    Spawn {
        /// What the Child Session's Agent should do.
        #[arg(long)]
        prompt: String,
        /// Attach the Child Session to this Plan Task.
        #[arg(long)]
        task: Option<TaskId>,
        /// The Child Session's Agent (default: this Session's).
        #[arg(long)]
        agent: Option<String>,
        /// The Child Session's Preset (default: this Session's). A looser one
        /// needs the user's approval.
        #[arg(long)]
        preset: Option<String>,
    },
    /// Say this Child Session is finished, handing its parent a summary.
    Done { summary: String },
    /// Block until each Child Session is Done or failed; prints the summaries.
    Wait {
        #[arg(required = true, num_args = 1..)]
        ids: Vec<String>,
    },
    /// Where Child Sessions stand, without blocking (default: all of them).
    Status {
        ids: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// Squash a Done Child Session onto this Session's Branch.
    Land { id: String },
    /// Discard a Child Session and its own children.
    Discard { id: String },
}

pub fn run(command: SessionCommand) -> ExitCode {
    let json = matches!(command, SessionCommand::Status { json: true, .. });
    let spawn = matches!(command, SessionCommand::Spawn { .. });
    let request = BrokerRequest::Session { op: op(command) };
    agent_call::call("session", request, |reply| match reply {
        BrokerReply::Session { children, .. } if json => {
            Ok(agent_call::print_json("session", &children))
        }
        BrokerReply::Session { said, children } => {
            if spawn {
                // The id alone on stdout, so scripts can capture it.
                if let Some(child) = children.first() {
                    println!("{}", child.id);
                }
                eprintln!("{said}");
            } else {
                println!("{said}");
            }
            Ok(ExitCode::SUCCESS)
        }
        other => Err(other),
    })
}

fn op(command: SessionCommand) -> SessionOp {
    match command {
        SessionCommand::Spawn {
            prompt,
            task,
            agent,
            preset,
        } => SessionOp::Spawn {
            prompt,
            task,
            agent,
            preset,
        },
        SessionCommand::Done { summary } => SessionOp::Done { summary },
        SessionCommand::Wait { ids } => SessionOp::Wait { ids },
        SessionCommand::Status { ids, .. } => SessionOp::Status { ids },
        SessionCommand::Land { id } => SessionOp::Land { id },
        SessionCommand::Discard { id } => SessionOp::Discard { id },
    }
}
