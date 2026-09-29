use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub(crate) async fn run(
    mut command: Command,
    input: Option<String>,
    limit: Duration,
    what: &str,
) -> Result<String, String> {
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
    let output = tokio::time::timeout(limit, child.wait_with_output())
        .await
        .map_err(|_| format!("{what} took longer than {}s", limit.as_secs()))?
        .map_err(|err| format!("{what} failed: {err}"))?;
    match output.status.success() {
        true => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
        false => Err(format!(
            "{what} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}
