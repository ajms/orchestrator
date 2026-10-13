use std::process::ExitCode;

use clap::Args;
use orch_daemon::{ProbeResult, probe_host_sandbox};
use orch_protocol::{Finding, ReconcileReport, Repair, Reply, Request};

use crate::client;

#[derive(Args)]
pub struct DoctorArgs {
    #[arg(long)]
    json: bool,
}

const FINDINGS: u8 = 1;
const FAILED: u8 = 2;

pub fn run(args: DoctorArgs) -> ExitCode {
    let reconciled = client::expect(Request::Reconcile, |reply| match reply {
        Reply::Reconciled { report } => Some(report),
        _ => None,
    });
    let report = match reconciled {
        Ok(report) => report,
        Err(err) => return failed(err),
    };
    let sandbox = match std::env::current_exe() {
        Ok(program) => probe_host_sandbox(program),
        Err(err) => Err(err.to_string()),
    };
    match args.json {
        true => match serde_json::to_string_pretty(&report) {
            Ok(json) => {
                println!("{json}");
                eprint!("{}", render_sandbox(&sandbox));
            }
            Err(err) => return failed(err),
        },
        false => print!("{}{}", render_sandbox(&sandbox), render(&report, &sandbox)),
    }
    match (report.findings().next(), &sandbox) {
        (None, Ok(_)) => ExitCode::SUCCESS,
        _ => ExitCode::from(FINDINGS),
    }
}

pub fn render_sandbox(sandbox: &ProbeResult) -> String {
    match sandbox {
        Ok(None) => "The Sandbox works, nested too.\n".into(),
        Ok(Some(warning)) => format!("The Sandbox works.\n  warning: {warning}\n"),
        Err(problem) => format!("! {problem}\n"),
    }
}

fn failed(message: impl std::fmt::Display) -> ExitCode {
    client::fail("doctor", FAILED, message)
}

pub fn render(report: &ReconcileReport, sandbox: &ProbeResult) -> String {
    let mut out = String::new();
    for repair in &report.repaired {
        out.push_str(&format!("Reconciled: {}\n", describe_repair(repair)));
    }
    let count = report.findings().count() + usize::from(sandbox.is_err());
    if count == 0 {
        out.push_str("No problems found.\n");
        return out;
    }
    for repo in report.repos.iter().filter(|repo| !repo.findings.is_empty()) {
        out.push_str(&format!("\n{}\n", repo.repo.display()));
        push_findings(&mut out, &repo.findings);
    }
    if !report.unknown_holders.is_empty() {
        out.push_str("\nHolders outside any Repo\n");
        push_findings(&mut out, &report.unknown_holders);
    }
    out.push_str(&format!(
        "\n{count} problem(s) found. Apply fixes from the TUI with :reconcile.\n"
    ));
    out
}

fn push_findings(out: &mut String, findings: &[Finding]) {
    for finding in findings {
        let problem = finding.problem.describe(|id| id.as_str().to_string());
        out.push_str(&format!("  ! {problem}\n"));
        for fix in &finding.fixes {
            out.push_str(&format!("      fix: {}\n", fix.label()));
        }
    }
}

fn describe_repair(repair: &Repair) -> String {
    match repair {
        Repair::AdoptedHolder { session } => {
            format!("adopted the live Holder of {}", session.as_str())
        }
        Repair::SuspendedSession { session } => {
            format!("{} is Suspended; its Agent was gone", session.as_str())
        }
        Repair::FinishedCleanup { session } => {
            format!("finished the cleanup of {}", session.as_str())
        }
    }
}
