use std::io::{BufRead, Read, Write};
use std::path::Path;
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use nix::sys::signal::{SigHandler, Signal, signal};
use nix::sys::termios::{LocalFlags, SetArg, cfmakeraw, tcgetattr, tcsetattr};
use orch_holder::SESSION_ENV;

use crate::subprocess::run_with_input;

pub fn run(script: Option<&Path>, agent_args: &[String]) -> ExitCode {
    if agent_args.iter().any(|arg| arg == "-p") {
        return draft(agent_args);
    }
    disable_echo();
    announce(agent_args);
    let scripted = match script.map(std::fs::read_to_string).transpose() {
        Ok(text) => text.unwrap_or_default(),
        Err(err) => {
            eprintln!("fake-agent: {err}");
            return ExitCode::FAILURE;
        }
    };
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

fn draft(agent_args: &[String]) -> ExitCode {
    let mut instruction = String::new();
    let _ = std::io::stdin().read_to_string(&mut instruction);
    let conversation = agent_args
        .iter()
        .skip_while(|arg| *arg != "--resume")
        .nth(1)
        .map_or("nothing", String::as_str);
    print!(
        "Drafted from {conversation}\n\nargs: {}\n{}\n",
        agent_args.join(" "),
        instruction.trim()
    );
    ExitCode::SUCCESS
}

fn announce(agent_args: &[String]) {
    let mut args = agent_args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--settings" => {
                if let Some(command) = args.next().and_then(|json| worktree_hook(json)) {
                    say(&format!("worktree-hook> {command}"));
                }
            }
            "--" => {
                let prompt: Vec<&str> = args.by_ref().map(String::as_str).collect();
                say(&format!("prompt> {}", prompt.join(" ")));
            }
            flag if flag.starts_with("--") => {
                let value = args.next().map_or("", String::as_str);
                say(&format!("{}> {value}", &flag[2..]));
            }
            other => say(&format!("arg> {other}")),
        }
    }
}

fn worktree_hook(settings: &str) -> Option<String> {
    let settings: serde_json::Value = serde_json::from_str(settings).ok()?;
    let command = &settings["hooks"]["WorktreeCreate"][0]["hooks"][0]["command"];
    command.as_str().map(String::from)
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
        "" => {}
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
            let (agent, payload) = match rest.strip_prefix("--agent ") {
                Some(tagged) => {
                    let (agent, payload) = tagged.split_once(' ').unwrap_or((tagged, ""));
                    (Some(agent), payload)
                }
                None => (None, rest),
            };
            let output = run_orch_subcommand(command, agent, payload);
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
        "exit-on-hangup" => exit_on_hangup(),
        "write" | "exists" | "save" => {
            let path = Path::new(rest);
            let outcome = match command {
                "write" => std::fs::write(path, "x").map(|()| "ok"),
                "save" => save(path).map(|()| "ok"),
                _ => Ok(if path.exists() { "yes" } else { "no" }),
            };
            let outcome = outcome.map_or_else(|err| err.to_string(), String::from);
            say(&format!("{line} -> {outcome}"));
        }
        "sh" => {
            let status = Command::new("sh")
                .args(["-c", rest])
                .stdin(std::process::Stdio::null())
                .output()
                .map(|output| output.status.code().unwrap_or(-1));
            match status {
                Ok(code) => say(&format!("{line} -> exit {code}")),
                Err(err) => say(&format!("{line} -> {err}")),
            }
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

static HUNG_UP: AtomicBool = AtomicBool::new(false);

extern "C" fn note_hangup(_: i32) {
    HUNG_UP.store(true, Ordering::Relaxed);
}

fn exit_on_hangup() {
    let _ = unsafe { signal(Signal::SIGHUP, SigHandler::Handler(note_hangup)) };
    std::thread::spawn(|| {
        while !HUNG_UP.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(10));
        }
        std::process::exit(0);
    });
    say("hangup handled");
}

fn save(path: &Path) -> std::io::Result<()> {
    let staged = path.with_extension("staged");
    std::fs::write(&staged, "saved")?;
    if std::fs::rename(&staged, path).is_err() {
        let _ = std::fs::remove_file(&staged);
        std::fs::write(path, "saved")?;
    }
    Ok(())
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

fn run_orch_subcommand(subcommand: &str, agent: Option<&str>, payload: &str) -> String {
    let session = std::env::var(SESSION_ENV).unwrap_or_default();
    let orch = std::env::current_exe().unwrap_or_else(|_| "orch".into());
    let mut command = Command::new(orch);
    command.args([subcommand, "--session", &session]);
    if let Some(agent) = agent {
        command.args(["--agent", agent]);
    }
    run_with_input(&mut command, payload.as_bytes(), None)
        .map(|output| String::from_utf8_lossy(&output).into_owned())
        .unwrap_or_else(|| "<failed>".into())
}
