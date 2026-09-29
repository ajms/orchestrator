use std::io::{Read, Write};
use std::process::{Command, ExitCode};
use std::time::Duration;

use orch_agent::{AgentAdapter, ClaudeCode};
use orch_config::xdg;
use orch_core::{AgentEvent, SessionId};
use orch_holder::{ToHolder, locate_socket, report};

use crate::subprocess::run_with_input;

const STATUSLINE_TIMEOUT: Duration = Duration::from_secs(3);

pub fn run(session: &str) -> ExitCode {
    let mut payload = String::new();
    if std::io::stdin().read_to_string(&mut payload).is_err() {
        return ExitCode::SUCCESS;
    }
    if let Ok(session) = SessionId::parse(session) {
        let tap = ToHolder::Tap {
            payload: payload.clone(),
        };
        let _ = report(&locate_socket(&session), &tap);
    }
    let claude = ClaudeCode::default();
    let line = std::env::current_dir()
        .ok()
        .and_then(|cwd| claude.user_statusline_command(&cwd, xdg::process_env))
        .and_then(|command| {
            run_with_input(
                Command::new("sh").args(["-c", &command]),
                payload.as_bytes(),
                Some(STATUSLINE_TIMEOUT),
            )
        })
        .unwrap_or_else(|| minimal_line(&claude, &payload).into_bytes());
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(&line);
    let _ = stdout.flush();
    ExitCode::SUCCESS
}

fn minimal_line(agent: &impl AgentAdapter, payload: &str) -> String {
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
