use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode};

use clap::Subcommand;
use orch_holder::{
    BrokerReply, BrokerRequest, GhOutput, HOLDER_SOCKET_ENV, RefUpdate, RemoteRef, request_broker,
};

const ORIGIN: &str = "origin";
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
    #[command(disable_help_flag = true)]
    SshKeygen {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

pub fn run(command: BrokerCommand) -> ExitCode {
    let Some(socket) = std::env::var_os(HOLDER_SOCKET_ENV).map(PathBuf::from) else {
        eprintln!("orch: this goes through the Broker, which only a sandboxed Session has");
        return ExitCode::FAILURE;
    };
    let url = match command {
        BrokerCommand::Gh { args } => return gh(&socket, args),
        BrokerCommand::SshKeygen { args } => return ssh_keygen(&socket, args),
        BrokerCommand::GitRemote { url, .. } => url.filter(|url| url != ORIGIN),
    };
    let ask = |request| request_broker(&socket, request);
    let stdin = io::stdin().lock();
    let helped = match url {
        Some(url) => read_only_helper(stdin, io::stdout().lock(), &url, ask),
        None => remote_helper(stdin, io::stdout().lock(), ask),
    };
    match helped {
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
    let reply = inline_body_file(args)
        .and_then(inline_api_input)
        .and_then(|(args, input)| request_broker(socket, BrokerRequest::Gh { args, input }));
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
            exit_code(Some(code))
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

fn ssh_keygen(socket: &std::path::Path, args: Vec<String>) -> ExitCode {
    let signs = args.windows(2).any(|pair| pair == ["-Y", "sign"]);
    let buffer = args.last().filter(|_| signs).map(PathBuf::from);
    let Some(buffer) = buffer else {
        return match Command::new("ssh-keygen").args(&args).status() {
            Ok(status) => exit_code(status.code()),
            Err(err) => {
                eprintln!("orch: running ssh-keygen: {err}");
                ExitCode::FAILURE
            }
        };
    };
    let reply = std::fs::read(&buffer)
        .and_then(|payload| request_broker(socket, BrokerRequest::Sign { payload }));
    let written = match reply {
        Ok(BrokerReply::Signed { signature }) => {
            let mut sig = buffer.into_os_string();
            sig.push(".sig");
            std::fs::write(sig, signature)
        }
        Ok(BrokerReply::Refused { reason }) => Err(io::Error::other(reason)),
        Ok(other) => Err(io::Error::other(format!(
            "the Broker answered out of turn: {other:?}"
        ))),
        Err(err) => Err(err),
    };
    match written {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("orch: {err}");
            ExitCode::FAILURE
        }
    }
}

fn exit_code(code: Option<i32>) -> ExitCode {
    ExitCode::from(code.and_then(|code| u8::try_from(code).ok()).unwrap_or(1))
}

fn inline_body_file(args: Vec<String>) -> io::Result<Vec<String>> {
    let takes_body = |command: &str, sub: &str| match command {
        "pr" => matches!(sub, "create" | "edit" | "comment" | "review"),
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

fn inline_api_input(args: Vec<String>) -> io::Result<(Vec<String>, Option<String>)> {
    if args.first().is_none_or(|command| command != "api") {
        return Ok((args, None));
    }
    let mut kept = Vec::with_capacity(args.len());
    let mut input = None;
    let mut rest = args.into_iter();
    while let Some(arg) = rest.next() {
        let path = match arg.as_str() {
            "--input" => rest.next(),
            _ => arg.strip_prefix("--input=").map(String::from),
        };
        let Some(path) = path else {
            kept.push(arg);
            continue;
        };
        if input.is_some() {
            return Err(io::Error::other("gh api takes one --input"));
        }
        input = Some(match path.as_str() {
            "-" => io::read_to_string(io::stdin())?,
            path => std::fs::read_to_string(path)?,
        });
    }
    Ok((kept, input))
}

fn remote_helper(
    input: impl BufRead,
    mut out: impl Write,
    ask: impl Fn(BrokerRequest) -> io::Result<BrokerReply>,
) -> io::Result<()> {
    let mut leases: HashMap<String, String> = HashMap::new();
    let mut batch: Vec<RefUpdate> = Vec::new();
    let mut wants: Vec<RemoteRef> = Vec::new();
    for line in input.lines() {
        let line = line?;
        let (command, rest) = line.split_once(' ').unwrap_or((&line, ""));
        match command {
            "capabilities" => write!(out, "option\nfetch\npush\n\n")?,
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
                let listing = match rest {
                    "for-push" => BrokerRequest::Refs,
                    _ => BrokerRequest::OriginRefs,
                };
                let refs = match ask(listing)? {
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
            "fetch" => {
                let (commit, name) = rest.split_once(' ').unwrap_or((rest, ""));
                wants.push(RemoteRef {
                    name: name.into(),
                    commit: commit.into(),
                });
            }
            "" if !wants.is_empty() => {
                match ask(BrokerRequest::Fetch {
                    wants: std::mem::take(&mut wants),
                })? {
                    BrokerReply::Done { .. } => writeln!(out)?,
                    BrokerReply::Refused { reason } => return Err(io::Error::other(reason)),
                    other => return Err(io::Error::other(format!("{other:?}"))),
                }
            }
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

fn read_only_helper(
    input: impl BufRead,
    mut out: impl Write,
    url: &str,
    ask: impl Fn(BrokerRequest) -> io::Result<BrokerReply>,
) -> io::Result<()> {
    let mut mirror: Option<String> = None;
    let mut wants: Vec<String> = Vec::new();
    let mut pushes: Vec<String> = Vec::new();
    for line in input.lines() {
        let line = line?;
        let (command, rest) = line.split_once(' ').unwrap_or((&line, ""));
        match command {
            "capabilities" => write!(out, "option\nfetch\npush\n\n")?,
            "option" => {
                let reply = match rest.split_once(' ') {
                    Some(("progress" | "verbosity", _)) => "ok",
                    _ => "unsupported",
                };
                writeln!(out, "{reply}")?;
            }
            "list" if rest == "for-push" => writeln!(out)?,
            "list" => {
                let path = match ask(BrokerRequest::Mirror { url: url.into() })? {
                    BrokerReply::Mirrored { path, .. } => path,
                    BrokerReply::Refused { reason } => return Err(io::Error::other(reason)),
                    other => return Err(io::Error::other(format!("{other:?}"))),
                };
                for listed in mirror_refs(&path)? {
                    writeln!(out, "{listed}")?;
                }
                writeln!(out)?;
                mirror = Some(path);
            }
            "fetch" => {
                let (_, name) = rest.split_once(' ').unwrap_or((rest, ""));
                wants.push(name.into());
            }
            "push" => {
                let dst = rest.rsplit(':').next().unwrap_or(rest);
                pushes.push(dst.into());
            }
            "" if !wants.is_empty() => {
                let path = mirror
                    .as_deref()
                    .ok_or_else(|| io::Error::other("fetch before list"))?;
                fetch_from_mirror(path, &std::mem::take(&mut wants))?;
                writeln!(out)?;
            }
            "" if !pushes.is_empty() => {
                for dst in std::mem::take(&mut pushes) {
                    writeln!(
                        out,
                        "error {dst} the Broker only reads {url}; it pushes only this Session's Branch to origin"
                    )?;
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

fn mirror_refs(path: &str) -> io::Result<Vec<String>> {
    let listing = Command::new("git")
        .args(["ls-remote", "--symref", path])
        .output()?;
    if !listing.status.success() {
        return Err(io::Error::other(format!(
            "listing the mirror failed: {}",
            String::from_utf8_lossy(&listing.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&listing.stdout)
        .lines()
        .filter_map(|line| match line.strip_prefix("ref: ") {
            Some(symref) => symref
                .split_once('\t')
                .map(|(target, name)| format!("@{target} {name}")),
            None => line
                .split_once('\t')
                .map(|(commit, name)| format!("{commit} {name}")),
        })
        .collect())
}

fn fetch_from_mirror(path: &str, names: &[String]) -> io::Result<()> {
    let fetched = Command::new("git")
        .args([
            "-c",
            "protocol.file.allow=always",
            "fetch",
            "-q",
            "--no-tags",
        ])
        .args(["--no-write-fetch-head", "--refmap=", path])
        .args(names)
        .stdout(std::process::Stdio::null())
        .status()?;
    match fetched.success() {
        true => Ok(()),
        false => Err(io::Error::other("fetching from the mirror failed")),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn speak(script: &str) -> (String, Vec<BrokerRequest>) {
        let asked = std::cell::RefCell::new(Vec::new());
        let mut out = Vec::new();
        let ask = |request: BrokerRequest| {
            asked.borrow_mut().push(request);
            Ok(BrokerReply::Refused {
                reason: "unexpected".into(),
            })
        };
        read_only_helper(
            script.as_bytes(),
            &mut out,
            "https://github.com/libs/core",
            ask,
        )
        .unwrap();
        (String::from_utf8(out).unwrap(), asked.into_inner())
    }

    #[test]
    fn the_read_only_helper_takes_progress_options_only() {
        let (said, _) = speak("capabilities\noption verbosity 1\noption depth 1\n\n");
        assert_eq!(said, "option\nfetch\npush\n\nok\nunsupported\n");
    }

    #[test]
    fn a_push_through_the_read_only_helper_is_refused_without_asking_the_broker() {
        let (said, asked) = speak("list for-push\npush refs/heads/main:refs/heads/main\n\n\n");
        assert_eq!(
            said,
            "\nerror refs/heads/main the Broker only reads https://github.com/libs/core; it pushes only this Session's Branch to origin\n\n"
        );
        assert!(asked.is_empty());
    }

    #[test]
    fn a_refused_listing_fails_the_read_only_helper() {
        let mut out = Vec::new();
        let ask = |_| {
            Ok(BrokerReply::Refused {
                reason: "not readable".into(),
            })
        };
        let failed = read_only_helper("list\n".as_bytes(), &mut out, "https://github.com/x/y", ask);
        assert_eq!(failed.unwrap_err().to_string(), "not readable");
    }
}
