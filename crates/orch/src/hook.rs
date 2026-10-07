use std::io::Read;
use std::process::ExitCode;

use orch_agent::{GuardAnswer, by_name};
use orch_core::SessionId;
use orch_holder::{ToHolder, locate_socket, report, request_guard};

pub fn run(agent: &str, session: Option<&str>) -> ExitCode {
    let mut payload = String::new();
    if std::io::stdin().read_to_string(&mut payload).is_err() {
        return ExitCode::SUCCESS;
    }
    let socket = session
        .and_then(|session| SessionId::parse(session).ok())
        .map(|session| locate_socket(&session));
    match by_name(agent).filter(|adapter| adapter.is_guard_payload(&payload)) {
        Some(adapter) => {
            let answer = match (session, socket) {
                (None, _) => GuardAnswer::Proceed,
                (Some(_), None) => GuardAnswer::Ask,
                (Some(_), Some(socket)) => request_guard(&socket, agent, &payload),
            };
            if let Some(output) = adapter.guard_answer(&answer) {
                println!("{output}");
            }
        }
        None => {
            if let Some(socket) = socket {
                let agent = agent.into();
                let _ = report(&socket, &ToHolder::Hook { agent, payload });
            }
        }
    }
    ExitCode::SUCCESS
}
