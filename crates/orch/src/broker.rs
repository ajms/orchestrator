use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use clap::Subcommand;
use orch_holder::{
    BrokerReply, BrokerRequest, GhOutput, HOLDER_SOCKET_ENV, RefUpdate, request_broker,
};

const ABSENT: &str = "0000000000000000000000000000000000000000";

#[derive(Subcommand)]
pub enum BrokerCommand {
    GitRemote {
        remote: String,
        url: Option<String>,
    },
    #[command(disable_help_flag = true)]
    Gh {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

pub fn run(command: BrokerCommand) -> ExitCode {
    let Some(socket) = std::env::var_os(HOLDER_SOCKET_ENV).map(PathBuf::from) else {
        eprintln!("orch: this goes through the Broker, which only a sandboxed Session has");
        return ExitCode::FAILURE;
    };
    if let BrokerCommand::Gh { args } = command {
        return gh(&socket, args);
    }
    let ask = |request| request_broker(&socket, request);
    let stdin = io::stdin().lock();
    match remote_helper(stdin, io::stdout().lock(), ask) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("orch: {err}");
            ExitCode::FAILURE
        }
    }
}

fn gh(socket: &std::path::Path, args: Vec<String>) -> ExitCode {
    if matches!(args.get(..2), Some([auth, token_or_status]) if auth == "auth" && matches!(token_or_status.as_str(), "token" | "status"))
    {
        eprintln!("You are not logged in to any GitHub hosts inside the Sandbox.");
        return ExitCode::FAILURE;
    }
    let reply =
        inline_body_file(args).and_then(|args| request_broker(socket, BrokerRequest::Gh { args }));
    match reply {
        Ok(BrokerReply::Gh {
            output:
                GhOutput {
                    stdout,
                    stderr,
                    code,
                },
            ..
        }) => {
            print!("{stdout}");
            eprint!("{stderr}");
            let _ = io::stdout().flush();
            ExitCode::from(u8::try_from(code).unwrap_or(1))
        }
        Ok(BrokerReply::Refused { reason }) => {
            eprintln!("orch: {reason}");
            ExitCode::FAILURE
        }
        Ok(other) => {
            eprintln!("orch: the Broker answered out of turn: {other:?}");
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("orch: {err}");
            ExitCode::FAILURE
        }
    }
}

fn inline_body_file(args: Vec<String>) -> io::Result<Vec<String>> {
    let takes_body = |command: &str, sub: &str| match command {
        "pr" => matches!(sub, "create" | "edit"),
        "issue" => matches!(sub, "create" | "comment" | "edit"),
        _ => false,
    };
    if !matches!(args.get(..2), Some([command, sub]) if takes_body(command, sub)) {
        return Ok(args);
    }
    let mut inlined = Vec::with_capacity(args.len());
    let mut rest = args.into_iter();
    while let Some(arg) = rest.next() {
        let path = match arg.as_str() {
            "--body-file" | "-F" => rest.next(),
            _ => arg
                .strip_prefix("--body-file=")
                .or_else(|| arg.strip_prefix("-F"))
                .map(String::from),
        };
        let Some(path) = path else {
            inlined.push(arg);
            continue;
        };
        let body = match path.as_str() {
            "-" => io::read_to_string(io::stdin())?,
            path => std::fs::read_to_string(path)?,
        };
        inlined.extend(["--body".into(), body]);
    }
    Ok(inlined)
}

fn remote_helper(
    input: impl BufRead,
    mut out: impl Write,
    ask: impl Fn(BrokerRequest) -> io::Result<BrokerReply>,
) -> io::Result<()> {
    let mut leases: HashMap<String, String> = HashMap::new();
    let mut batch: Vec<RefUpdate> = Vec::new();
    for line in input.lines() {
        let line = line?;
        let (command, rest) = line.split_once(' ').unwrap_or((&line, ""));
        match command {
            "capabilities" => write!(out, "option\npush\n\n")?,
            "option" => {
                let reply = match rest.split_once(' ') {
                    Some(("cas", lease)) => {
                        if let Some((name, expected)) = lease.split_once(':') {
                            leases.insert(name.into(), expected.into());
                        }
                        "ok"
                    }
                    Some(("progress" | "verbosity", _)) => "ok",
                    _ => "unsupported",
                };
                writeln!(out, "{reply}")?;
            }
            "list" => {
                let refs = match ask(BrokerRequest::Refs)? {
                    BrokerReply::Refs { refs } => refs,
                    BrokerReply::Refused { reason } => return Err(io::Error::other(reason)),
                    other => return Err(io::Error::other(format!("{other:?}"))),
                };
                for listed in refs {
                    writeln!(out, "{} {}", listed.commit, listed.name)?;
                }
                writeln!(out)?;
            }
            "push" => batch.push(ref_update(rest, &leases)),
            "" if !batch.is_empty() => {
                let updates = std::mem::take(&mut batch);
                let dsts: Vec<String> = updates.iter().map(|update| update.dst.clone()).collect();
                let outcome = match ask(BrokerRequest::Push { updates })? {
                    BrokerReply::Done { .. } => None,
                    BrokerReply::Refused { reason } => Some(reason),
                    _ => Some("the Broker answered out of turn".into()),
                };
                for dst in dsts {
                    match &outcome {
                        None => writeln!(out, "ok {dst}")?,
                        Some(reason) => writeln!(out, "error {dst} {}", one_line(reason))?,
                    }
                }
                writeln!(out)?;
            }
            "" => return Ok(()),
            _ => return Err(io::Error::other(format!("unexpected command {line:?}"))),
        }
        out.flush()?;
    }
    Ok(())
}

fn ref_update(spec: &str, leases: &HashMap<String, String>) -> RefUpdate {
    let force = spec.starts_with('+');
    let (src, dst) = spec
        .trim_start_matches('+')
        .split_once(':')
        .unwrap_or((spec, spec));
    let lease = leases.get(dst).map(|expected| match expected.as_str() {
        ABSENT => String::new(),
        expected => expected.into(),
    });
    RefUpdate {
        src: src.into(),
        commit: (!src.is_empty()).then(|| resolve(src)).flatten(),
        dst: dst.into(),
        force,
        lease,
    }
}

fn resolve(src: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("{src}^{{commit}}"))
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn one_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}
