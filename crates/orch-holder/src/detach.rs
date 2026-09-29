use std::fs::File;
use std::io::{self, Read, Write};

use nix::fcntl::OFlag;
use nix::unistd::{ForkResult, fork, setsid};

use crate::HoldConfig;
use crate::holder::serve;

const LOG_ENV: &str = "ORCH_HOLDER_LOG";

pub fn hold_detached(config: HoldConfig) -> io::Result<u32> {
    debug_assert_eq!(
        thread_count(),
        1,
        "fork must happen before any thread starts"
    );
    let (ready_read, ready_write) = nix::unistd::pipe2(OFlag::O_CLOEXEC)?;
    match unsafe { fork() }? {
        ForkResult::Parent { child } => {
            drop(ready_write);
            let mut report = String::new();
            File::from(ready_read).read_to_string(&mut report)?;
            nix::sys::wait::waitpid(child, None)?;
            match report.strip_prefix("ok ") {
                Some(pid) => pid.trim().parse().map_err(io::Error::other),
                None if report.is_empty() => Err(io::Error::other("holder failed to start")),
                None => Err(io::Error::other(report)),
            }
        }
        ForkResult::Child => {
            drop(ready_read);
            let mut ready = Some(File::from(ready_write));
            let result = detach().and_then(|()| {
                serve(config, || {
                    if let Some(mut pipe) = ready.take() {
                        let _ = writeln!(pipe, "ok {}", std::process::id());
                    }
                })
            });
            if let (Err(err), Some(mut pipe)) = (&result, ready.take()) {
                let _ = write!(pipe, "{err}");
            }
            std::process::exit(i32::from(result.is_err()));
        }
    }
}

fn thread_count() -> usize {
    std::fs::read_dir("/proc/self/task").map_or(1, Iterator::count)
}

fn detach() -> io::Result<()> {
    setsid()?;
    if let ForkResult::Parent { .. } = unsafe { fork() }? {
        std::process::exit(0);
    }
    let null = File::options().read(true).write(true).open("/dev/null")?;
    let log = match std::env::var_os(LOG_ENV) {
        Some(path) => File::options().create(true).append(true).open(path)?,
        None => null.try_clone()?,
    };
    nix::unistd::dup2_stdin(&null)?;
    nix::unistd::dup2_stdout(&null)?;
    nix::unistd::dup2_stderr(&log)?;
    Ok(())
}
