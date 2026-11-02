use std::process::Stdio;
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

pub(crate) fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

pub(crate) async fn run(
    command: Command,
    input: Option<Vec<u8>>,
    limit: Duration,
    what: &str,
) -> Result<String, String> {
    let output = output(command, input, limit, what).await?;
    match output.status.success() {
        true => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
        false => Err(format!(
            "{what} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

pub(crate) async fn output(
    mut command: Command,
    input: Option<Vec<u8>>,
    limit: Duration,
    what: &str,
) -> Result<std::process::Output, String> {
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
    let stdin = child.stdin.take();
    let feed = async move {
        if let (Some(input), Some(mut stdin)) = (input, stdin) {
            let _ = stdin.write_all(&input).await;
        }
    };
    let run = async { tokio::join!(feed, child.wait_with_output()).1 };
    tokio::time::timeout(limit, run)
        .await
        .map_err(|_| format!("{what} took longer than {}s", limit.as_secs_f32()))?
        .map_err(|err| format!("{what} failed: {err}"))
}
