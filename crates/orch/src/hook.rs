use std::io::Read;
use std::process::ExitCode;

use orch_agent::{AgentAdapter, CLAUDE, ClaudeCode, GuardAnswer};
use orch_core::SessionId;
use orch_holder::{ToHolder, locate_socket, report, request_guard};

pub fn run(session: &str, agent: &str) -> ExitCode {
    let mut payload = String::new();
    if std::io::stdin().read_to_string(&mut payload).is_err() {
        return ExitCode::SUCCESS;
    }
    let adapter = (agent == CLAUDE).then(ClaudeCode::default);
    let socket = SessionId::parse(session)
        .ok()
        .map(|session| locate_socket(&session));
    if let Some(adapter) = adapter.filter(|adapter| adapter.is_guard_payload(&payload)) {
        let answer = socket.map_or(GuardAnswer::Ask, |socket| {
            request_guard(&socket, agent, &payload)
        });
        if let Some(output) = adapter.guard_answer(&answer) {
            println!("{output}");
        }
    } else if let Some(socket) = socket {
        let hook = ToHolder::Hook {
            payload,
            agent: agent.into(),
        };
        let _ = report(&socket, &hook);
    }
    ExitCode::SUCCESS
}
