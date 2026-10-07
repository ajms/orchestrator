use std::process::{Output, Stdio};
use std::time::Duration;

use orch_git::ENV_REDIRECTING_GIT;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub(crate) fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    for key in ENV_REDIRECTING_GIT {
        command.env_remove(key);
    }
    command
}

pub(crate) async fn run(
    command: Command,
    input: Option<String>,
    limit: Duration,
    what: &str,
) -> Result<String, String> {
    let output = output(command, input, limit, what).await?;
    match output.status.success() {
        true => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
        false => Err(failed(what, &output)),
    }
}

pub(crate) fn failed(what: &str, output: &Output) -> String {
    format!(
        "{what} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

pub(crate) async fn output(
    mut command: Command,
    input: Option<String>,
    limit: Duration,
    what: &str,
) -> Result<Output, String> {
    let stdin = match input {
        Some(_) => Stdio::piped(),
        None => Stdio::null(),
    };
    let mut child = command
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| format!("cannot run {what}: {err}"))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        let _ = stdin.write_all(input.as_bytes()).await;
    }
    tokio::time::timeout(limit, child.wait_with_output())
        .await
        .map_err(|_| format!("{what} took longer than {}s", limit.as_secs()))?
        .map_err(|err| format!("{what} failed: {err}"))
}
