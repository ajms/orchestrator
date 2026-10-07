use std::io::Read;
use std::process::ExitCode;

use orch_agent::{GuardAnswer, by_name};
use orch_core::SessionId;
use orch_holder::{ToHolder, locate_socket, report, request_guard};

pub fn run(agent: &str, session: &str) -> ExitCode {
    let mut payload = String::new();
    if std::io::stdin().read_to_string(&mut payload).is_err() {
        return ExitCode::SUCCESS;
    }
    let socket = SessionId::parse(session)
        .ok()
        .map(|session| locate_socket(&session));
    match by_name(agent).filter(|adapter| adapter.is_guard_payload(&payload)) {
        Some(adapter) => {
            let answer = socket.map_or(GuardAnswer::Ask, |socket| {
                request_guard(&socket, agent, &payload)
            });
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
