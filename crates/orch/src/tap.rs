use std::io::{Read, Write};
use std::process::{Command, ExitCode};
use std::time::Duration;

use orch_agent::{AgentAdapter, by_name};
use orch_config::xdg;
use orch_core::{AgentEvent, SessionId};
use orch_holder::{ToHolder, locate_socket, report};

use crate::subprocess::run_with_input;

const STATUSLINE_TIMEOUT: Duration = Duration::from_secs(3);

pub fn run(agent: &str, session: Option<&str>) -> ExitCode {
    let mut payload = String::new();
    if std::io::stdin().read_to_string(&mut payload).is_err() {
        return ExitCode::SUCCESS;
    }
    if let Some(Ok(session)) = session.map(SessionId::parse) {
        let tap = ToHolder::Tap {
            agent: agent.into(),
            payload: payload.clone(),
        };
        let _ = report(&locate_socket(&session), &tap);
    }
    let Some(adapter) = by_name(agent) else {
        return ExitCode::SUCCESS;
    };
    let line = std::env::current_dir()
        .ok()
        .and_then(|cwd| adapter.user_statusline_command(&cwd, &xdg::process_env))
        .and_then(|command| {
            run_with_input(
                Command::new("sh").args(["-c", &command]),
                payload.as_bytes(),
                Some(STATUSLINE_TIMEOUT),
            )
        })
        .unwrap_or_else(|| match adapter.hookup() {
            Some(_) => Vec::new(),
            None => minimal_line(adapter.as_ref(), &payload).into_bytes(),
        });
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(&line);
    let _ = stdout.flush();
    ExitCode::SUCCESS
}

fn minimal_line(agent: &dyn AgentAdapter, payload: &str) -> String {
    let sample = agent
        .map_tap(payload)
        .ok()
        .into_iter()
        .flatten()
        .find_map(|event| match event {
            AgentEvent::UsageSample(sample) => Some(sample),
            _ => None,
        });
    let model = sample
        .as_ref()
        .and_then(|sample| sample.model.clone())
        .unwrap_or_else(|| "?".into());
    let context = sample
        .and_then(|sample| sample.context_used_percent)
        .map_or_else(|| "?".into(), |percent| format!("{percent:.0}"));
    format!("{model} · {context}%\n")
}
