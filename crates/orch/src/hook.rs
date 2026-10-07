use std::io::Read;
use std::process::ExitCode;

use orch_agent::hook_event::tag_hook_event;
use orch_agent::{GuardAnswer, by_name};
use orch_core::SessionId;
use orch_holder::{ToHolder, locate_socket, report, request_guard};

pub fn run(agent: &str, session: Option<&str>, event: Option<&str>) -> ExitCode {
    let adapter = by_name(agent);
    let fallback = || {
        if let Some(reply) = adapter
            .as_ref()
            .and_then(|adapter| adapter.fallback_hook_reply())
        {
            println!("{reply}");
        }
    };
    let mut payload = String::new();
    if std::io::stdin().read_to_string(&mut payload).is_err() {
        fallback();
        return ExitCode::SUCCESS;
    }
    if let Some(event) = event {
        payload = tag_hook_event(&payload, event);
    }
    let socket = session
        .and_then(|session| SessionId::parse(session).ok())
        .map(|session| locate_socket(&session));
    match adapter
        .as_ref()
        .filter(|adapter| adapter.is_guard_payload(&payload))
    {
        Some(adapter) => {
            let answer = match (session, socket) {
                (None, _) => GuardAnswer::Proceed,
                (Some(_), None) => GuardAnswer::Ask,
                (Some(_), Some(socket)) => request_guard(&socket, agent, &payload),
            };
            match adapter.guard_answer(&answer) {
                Some(output) => println!("{output}"),
                None => fallback(),
            }
        }
        None if serde_json::from_str::<serde_json::Value>(&payload).is_err() => fallback(),
        None => {
            if let Some(socket) = socket {
                let agent = agent.into();
                let _ = report(&socket, &ToHolder::Hook { agent, payload });
            }
        }
    }
    ExitCode::SUCCESS
}
