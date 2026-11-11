use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use orch_core::{Attachment, Plan, PlanOp, Readiness, TaskId, TaskStatus, join_ids};
use orch_holder::{BrokerReply, BrokerRequest, HOLDER_SOCKET_ENV, request_broker};

/// Writes the Plan of the Session this runs in. Only the Session's Agent
/// reaches it: the request goes through the Session's Holder socket.
#[derive(Subcommand)]
pub enum PlanCommand {
    /// Add a Task, at the end or after another Task.
    Add {
        title: String,
        /// Place the new Task right after this Task.
        #[arg(long)]
        after: Option<TaskId>,
        /// A Task the new Task waits for (repeatable).
        #[arg(long = "blocked-by")]
        blocked_by: Vec<TaskId>,
        /// An issue in any tracker: a URL or a reference such as #241.
        #[arg(long)]
        link: Option<String>,
    },
    /// Set a Task's status: pending, in-progress, done or dropped.
    Status {
        task: TaskId,
        #[arg(value_parser = parse_status)]
        status: TaskStatus,
    },
    /// Mark a Task as blocked by other Tasks.
    Block {
        task: TaskId,
        #[arg(long, required = true, num_args = 1..)]
        by: Vec<TaskId>,
    },
    /// Remove "blocked by" edges from a Task.
    Unblock {
        task: TaskId,
        #[arg(long, required = true, num_args = 1..)]
        by: Vec<TaskId>,
    },
    /// Link a Task to an issue; an empty reference removes the link.
    Link { task: TaskId, reference: String },
    /// Attach a Subagent to the Task it works on.
    Attach { task: TaskId, subagent: String },
    /// Show the Plan.
    Show {
        #[arg(long)]
        json: bool,
    },
}

fn parse_status(text: &str) -> Result<TaskStatus, String> {
    TaskStatus::parse(text).ok_or_else(|| "expected pending, in-progress, done or dropped".into())
}

pub fn run(command: PlanCommand) -> ExitCode {
    let Some(socket) = std::env::var_os(HOLDER_SOCKET_ENV).map(PathBuf::from) else {
        eprintln!("orch plan: run this inside an orch Session");
        return ExitCode::FAILURE;
    };
    let json = matches!(command, PlanCommand::Show { json: true });
    match request_broker(&socket, BrokerRequest::Plan { op: op(command) }) {
        Ok(BrokerReply::Plan { plan, said }) => {
            if json {
                match serde_json::to_string_pretty(&plan) {
                    Ok(text) => println!("{text}"),
                    Err(err) => {
                        eprintln!("orch plan: {err}");
                        return ExitCode::FAILURE;
                    }
                }
            } else if said.is_empty() {
                print!("{}", render(&plan));
            } else {
                println!("{said}");
            }
            ExitCode::SUCCESS
        }
        Ok(BrokerReply::Refused { reason }) => {
            eprintln!("orch plan: {reason}");
            ExitCode::FAILURE
        }
        Ok(other) => {
            eprintln!("orch plan: the Daemon answered out of turn: {other:?}");
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("orch plan: {err}");
            ExitCode::FAILURE
        }
    }
}

fn op(command: PlanCommand) -> PlanOp {
    match command {
        PlanCommand::Add {
            title,
            after,
            blocked_by,
            link,
        } => PlanOp::Add {
            title,
            after,
            blocked_by,
            link,
        },
        PlanCommand::Status { task, status } => PlanOp::Status { task, status },
        PlanCommand::Block { task, by } => PlanOp::Block { task, by },
        PlanCommand::Unblock { task, by } => PlanOp::Unblock { task, by },
        PlanCommand::Link { task, reference } => PlanOp::Link {
            task,
            link: Some(reference).filter(|link| !link.trim().is_empty()),
        },
        PlanCommand::Attach { task, subagent } => PlanOp::Attach {
            task,
            attachment: Attachment::Subagent(subagent),
        },
        PlanCommand::Show { .. } => PlanOp::Show,
    }
}

fn render(plan: &Plan) -> String {
    if plan.is_empty() {
        return "The Plan has no Tasks yet; add one with `orch plan add <title>`.\n".into();
    }
    let (done, total) = plan.progress();
    let mut out = format!("Plan: {done}/{total} done\n");
    for task in &plan.tasks {
        let mut line = format!("{:>3}. [{}] {}", task.id, task.status.name(), task.title);
        match plan.readiness(task) {
            Readiness::Settled => {}
            Readiness::Ready => line.push_str(" (ready)"),
            Readiness::BlockedBy(open) => {
                line.push_str(&format!(" (blocked by {})", join_ids(&open)))
            }
        }
        if let Some(link) = &task.link {
            line.push_str(&format!(" link: {link}"));
        }
        let attached = |child: bool| {
            task.attachments
                .iter()
                .filter(|attachment| matches!(attachment, Attachment::ChildSession(_)) == child)
                .map(Attachment::id)
                .collect::<Vec<_>>()
        };
        let subagents = attached(false);
        if !subagents.is_empty() {
            line.push_str(&format!(" subagents: {}", subagents.join(", ")));
        }
        let children = attached(true);
        if !children.is_empty() {
            line.push_str(&format!(" child sessions: {}", children.join(", ")));
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_lists_tasks_with_status_readiness_link_and_subagents() {
        let mut plan = Plan::default();
        for (title, blocked_by) in [("design", vec![]), ("build", vec![1])] {
            plan.apply(PlanOp::Add {
                title: title.into(),
                after: None,
                blocked_by,
                link: None,
            })
            .unwrap();
        }
        plan.apply(op(PlanCommand::Link {
            task: 2,
            reference: "#241".into(),
        }))
        .unwrap();
        plan.apply(op(PlanCommand::Attach {
            task: 2,
            subagent: "sub-1".into(),
        }))
        .unwrap();
        assert_eq!(
            render(&plan),
            "Plan: 0/2 done\n  1. [pending] design (ready)\n  2. [pending] build (blocked by 1) link: #241 subagents: sub-1\n"
        );
    }

    #[test]
    fn an_empty_reference_clears_the_link() {
        let cleared = op(PlanCommand::Link {
            task: 1,
            reference: " ".into(),
        });
        assert_eq!(
            cleared,
            PlanOp::Link {
                task: 1,
                link: None
            }
        );
    }

    #[test]
    fn statuses_parse_from_their_cli_names() {
        assert_eq!(parse_status("in-progress"), Ok(TaskStatus::InProgress));
        assert!(parse_status("finished").is_err());
    }
}
