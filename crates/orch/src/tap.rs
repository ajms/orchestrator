use std::io::{Read, Write};
use std::process::{Command, ExitCode};
use std::time::Duration;

use orch_agent::by_name;
use orch_config::xdg;
use orch_core::SessionId;
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
        .unwrap_or_else(|| adapter.fallback_statusline(&payload).into_bytes());
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(&line);
    let _ = stdout.flush();
    ExitCode::SUCCESS
}
