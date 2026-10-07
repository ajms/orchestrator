use std::io::{BufRead, Read, Write};
use std::path::Path;
use std::process::{Command, ExitCode};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use nix::sys::signal::{SigHandler, Signal, signal};
use nix::sys::termios::{LocalFlags, SetArg, cfmakeraw, tcgetattr, tcsetattr};
use orch_holder::SESSION_ENV;
use serde_json::{Value, json};

use crate::subprocess::run_with_input;

pub fn run(script: Option<&Path>, agent_args: &[String]) -> ExitCode {
    let scripted = match script.map(std::fs::read_to_string).transpose() {
        Ok(text) => text.unwrap_or_default(),
        Err(err) => {
            eprintln!("fake-agent: {err}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(request) = DraftRequest::find(&scripted, agent_args) {
        return request.answer(agent_args);
    }
    disable_echo();
    announce(agent_args);
    for line in scripted.lines() {
        if let Some(code) = execute(line) {
            return code;
        }
    }
    let stdin = std::io::stdin();
    let mut line = String::new();
    while stdin.lock().read_line(&mut line).is_ok_and(|read| read > 0) {
        if let Some(code) = execute(line.trim_end_matches('\n')) {
            return code;
        }
        line.clear();
    }
    ExitCode::SUCCESS
}

struct DraftRequest<'a> {
    conversation_flag: Option<&'a str>,
    error: Option<&'a str>,
}

impl<'a> DraftRequest<'a> {
    fn find(script: &'a str, agent_args: &[String]) -> Option<Self> {
        script.lines().find_map(|line| {
            let mut words = line.strip_prefix("draft ")?.split_whitespace();
            let flag = words.next()?;
            agent_args.iter().any(|arg| arg == flag).then(|| Self {
                conversation_flag: words.next(),
                error: script
                    .lines()
                    .find_map(|line| line.strip_prefix("draft-error ")),
            })
        })
    }

    fn answer(&self, agent_args: &[String]) -> ExitCode {
        let mut input = String::new();
        let _ = std::io::stdin().read_to_string(&mut input);
        let stream_json = |flag: &str| {
            agent_args
                .windows(2)
                .any(|pair| pair[0] == flag && pair[1] == "stream-json")
        };
        let instruction = match stream_json("--input-format") {
            true => user_text(&input),
            false => input,
        };
        let conversation = agent_args
            .iter()
            .skip_while(|arg| Some(arg.as_str()) != self.conversation_flag)
            .nth(1)
            .map_or("nothing", String::as_str);
        let session = std::env::var(SESSION_ENV).unwrap_or_else(|_| "<unset>".into());
        let drafted = format!(
            "Drafted from {conversation}\n\nargs: {}\n{SESSION_ENV}={session}\n{}\n",
            agent_args.join(" "),
            instruction.trim()
        );
        if !stream_json("--output-format") {
            print!("{drafted}");
            return ExitCode::SUCCESS;
        }
        let result = match self.error {
            Some(error) => json!({ "status": "ERROR", "error": error }),
            None => json!({ "status": "SUCCESS", "response": drafted }),
        };
        println!("{}", json!({ "event": "init", "conversation_id": "draft" }));
        println!("{}", json!({ "event": "result", "result": result }));
        match self.error {
            Some(_) => {
                eprintln!("fake-agent: the draft failed");
                ExitCode::FAILURE
            }
            None => ExitCode::SUCCESS,
        }
    }
}

fn user_text(input: &str) -> String {
    input
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|event| event["event"] == "user")
        .flat_map(|event| event["message"]["content"].as_array().cloned())
        .flatten()
        .filter_map(|part| part["text"].as_str().map(String::from))
        .collect()
}

fn announce(agent_args: &[String]) {
    let mut args = agent_args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--" => {
                let prompt: Vec<&str> = args.by_ref().map(String::as_str).collect();
                say(&format!("prompt> {}", prompt.join(" ")));
            }
            flag if flag.len() > 1 && flag.starts_with('-') => {
                let value = elide_json(args.next().map_or("", String::as_str));
                say(&format!("{}> {value}", flag.trim_start_matches('-')));
            }
            other => say(&format!("arg> {other}")),
        }
    }
}

fn elide_json(value: &str) -> &str {
    if value.starts_with('{') {
        "{…}"
    } else {
        value
    }
}

fn disable_echo() {
    let stdin = std::io::stdin();
    if let Ok(mut termios) = tcgetattr(&stdin) {
        termios.local_flags.remove(LocalFlags::ECHO);
        let _ = tcsetattr(&stdin, SetArg::TCSANOW, &termios);
    }
}

fn execute(line: &str) -> Option<ExitCode> {
    let line = line.trim_end_matches('\r');
    let (command, rest) = line.split_once(' ').unwrap_or((line, ""));
    match command {
        "" | "draft" | "draft-error" => {}
        "print" => say(&unescape(rest)),
        "lines" => {
            let (count, prefix) = rest.split_once(' ').unwrap_or((rest, "line"));
            for i in 0..count.parse().unwrap_or(0) {
                say(&format!("{prefix} {i}"));
            }
        }
        "env" => say(&format!(
            "{rest}={}",
            std::env::var(rest).unwrap_or_else(|_| "<unset>".into())
        )),
        "hook" | "tap" => {
            let output = run_orch_subcommand(command, rest);
            say(&format!("{command}> {}", output.trim_end()));
        }
        "flood" => {
            let line = "=".repeat(60);
            let mut count = 0_u64;
            loop {
                say(&format!("flood {count} {line}"));
                count += 1;
            }
        }
        "ignore-hangup" => {
            let _ = unsafe { signal(Signal::SIGHUP, SigHandler::SigIgn) };
            say("hangup ignored");
        }
        "size" => {
            let _ = Command::new("stty").arg("size").status();
            let _ = std::io::stdout().flush();
        }
        "mouse" => enable_mouse(rest),
        "raw" => echo_raw(),
        "osc52" => copy_to_clipboard(rest),
        "sleep" => std::thread::sleep(Duration::from_millis(rest.parse().unwrap_or(0))),
        "exit" => return Some(ExitCode::from(rest.parse::<u8>().unwrap_or(0))),
        _ => say(&format!("unknown> {}", line.escape_debug())),
    }
    None
}

fn enable_mouse(request: &str) {
    let (mode, encoding) = request.split_once(' ').unwrap_or((request, "default"));
    let mode = match mode {
        "press" => "9",
        "press-release" => "1000",
        "button-motion" => "1002",
        "any-motion" => "1003",
        _ => return say(&format!("unknown mouse mode> {mode}")),
    };
    let encoding = match encoding {
        "default" => "",
        "utf8" => "\x1b[?1005h",
        "sgr" => "\x1b[?1006h",
        _ => return say(&format!("unknown mouse encoding> {encoding}")),
    };
    emit(&format!("\x1b[?{mode}h{encoding}"));
}

fn echo_raw() {
    let stdin = std::io::stdin();
    let Ok(cooked) = tcgetattr(&stdin) else {
        return say("raw> unavailable");
    };
    let mut raw = cooked.clone();
    cfmakeraw(&mut raw);
    let _ = tcsetattr(&stdin, SetArg::TCSANOW, &raw);
    say("raw on");
    let mut buf = [0; 1024];
    while let Ok(n @ 1..) = stdin.lock().read(&mut buf) {
        let received = &buf[..n];
        if received == b"\x04" {
            break;
        }
        say(&format!("raw> {}", received.escape_ascii()));
    }
    let _ = tcsetattr(&stdin, SetArg::TCSANOW, &cooked);
    say("raw off");
}

fn copy_to_clipboard(text: &str) {
    emit(&format!("\x1b]52;c;{}\x07", STANDARD.encode(text)));
}

fn emit(sequence: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(sequence.as_bytes());
    let _ = stdout.flush();
}

fn say(text: &str) {
    let mut stdout = std::io::stdout().lock();
    let _ = write!(stdout, "{text}\r\n");
    let _ = stdout.flush();
}

fn unescape(text: &str) -> String {
    text.replace("\\e", "\x1b")
}

fn run_orch_subcommand(subcommand: &str, rest: &str) -> String {
    let session = std::env::var(SESSION_ENV).unwrap_or_default();
    let orch = std::env::current_exe().unwrap_or_else(|_| "orch".into());
    let mut command = Command::new(orch);
    command.args([subcommand, "--session", &session]);
    let mut payload = rest;
    while let Some(flagged) = payload.strip_prefix("--") {
        let (flag, after) = flagged.split_once(' ').unwrap_or((flagged, ""));
        let (value, after) = after.split_once(' ').unwrap_or((after, ""));
        command.arg(format!("--{flag}")).arg(value);
        payload = after;
    }
    run_with_input(&mut command, payload.as_bytes(), None)
        .map(|output| String::from_utf8_lossy(&output).into_owned())
        .unwrap_or_else(|| "<failed>".into())
}
