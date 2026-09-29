use std::io::Read;
use std::process::ExitCode;

use orch_agent::{AgentAdapter, ClaudeCode, GuardAnswer};
use orch_core::SessionId;
use orch_holder::{ToHolder, locate_socket, report, request_guard};

pub fn run(session: &str) -> ExitCode {
    let mut payload = String::new();
    if std::io::stdin().read_to_string(&mut payload).is_err() {
        return ExitCode::SUCCESS;
    }
    let agent = ClaudeCode::default();
    let socket = SessionId::parse(session)
        .ok()
        .map(|session| locate_socket(&session));
    if agent.is_guard_payload(&payload) {
        let answer = socket.map_or(GuardAnswer::Ask, |socket| request_guard(&socket, &payload));
        if let Some(output) = agent.guard_answer(&answer) {
            println!("{output}");
        }
    } else if let Some(socket) = socket {
        let _ = report(&socket, &ToHolder::Hook { payload });
    }
    ExitCode::SUCCESS
}
