//! The plumbing `orch plan` and `orch session` share: they run inside a
//! Session, send one request through its Holder socket and print the answer.

use std::path::PathBuf;
use std::process::ExitCode;

use orch_holder::{BrokerReply, BrokerRequest, HOLDER_SOCKET_ENV, request_broker};
use serde::Serialize;

/// Sends `request` and hands `answer` the reply. A missing socket, a
/// refusal, a transport error and a reply `answer` doesn't expect (it
/// hands it back) are reported as `orch <command>: …` and fail.
pub fn call(
    command: &str,
    request: BrokerRequest,
    answer: impl FnOnce(BrokerReply) -> Result<ExitCode, BrokerReply>,
) -> ExitCode {
    let Some(socket) = std::env::var_os(HOLDER_SOCKET_ENV).map(PathBuf::from) else {
        return fail(command, "run this inside an orch Session");
    };
    match request_broker(&socket, request) {
        Ok(BrokerReply::Refused { reason }) => fail(command, reason),
        Ok(reply) => match answer(reply) {
            Ok(code) => code,
            Err(other) => fail(
                command,
                format!("the Daemon answered out of turn: {other:?}"),
            ),
        },
        Err(err) => fail(command, err),
    }
}

/// Prints `value` as pretty JSON.
pub fn print_json(command: &str, value: &impl Serialize) -> ExitCode {
    match serde_json::to_string_pretty(value) {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(err) => fail(command, err),
    }
}

fn fail(command: &str, message: impl std::fmt::Display) -> ExitCode {
    eprintln!("orch {command}: {message}");
    ExitCode::FAILURE
}
