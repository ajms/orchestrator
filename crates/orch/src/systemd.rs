use std::path::{Path, PathBuf};
use std::process::ExitCode;

use orch_config::xdg;

const UNIT: &str = orch_protocol::DAEMON_UNIT;
const NO_CONFIG_HOME: &str = "neither XDG_CONFIG_HOME nor HOME is set";

fn unit_dir() -> Option<PathBuf> {
    Some(xdg::config_home(xdg::process_env)?.join("systemd/user"))
}

fn unit(program: &Path, search_path: Option<&str>) -> String {
    let program = exec_word(&program.display().to_string());
    let environment = search_path
        .map(|path| format!("Environment={}\n", environment_assignment("PATH", path)))
        .unwrap_or_default();
    format!(
        "[Unit]\n\
         Description=orch Daemon: runs coding-agent Sessions\n\
         \n\
         [Service]\n\
         ExecStart={program} daemon --no-idle-exit\n\
         {environment}\
         Restart=on-failure\n\
         KillMode=process\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    )
}

fn exec_word(word: &str) -> String {
    let needs_quotes = word.is_empty()
        || word
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | ';'));
    let escaped = specifiers(&word.replace('$', "$$"));
    match needs_quotes {
        true => quoted(&escaped),
        false => escaped,
    }
}

fn environment_assignment(key: &str, value: &str) -> String {
    quoted(&specifiers(&format!("{key}={value}")))
}

fn specifiers(text: &str) -> String {
    text.replace('%', "%%")
}

fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn install() -> ExitCode {
    let Some(dir) = unit_dir() else {
        return failed(NO_CONFIG_HOME);
    };
    let path = dir.join(UNIT);
    let program = match std::env::current_exe().and_then(|exe| exe.canonicalize()) {
        Ok(program) => program,
        Err(err) => return failed(format!("cannot locate the orch binary: {err}")),
    };
    let search_path = std::env::var("PATH").ok();
    let written = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(&path, unit(&program, search_path.as_deref())));
    if let Err(err) = written {
        return failed(format!("{}: {err}", path.display()));
    }
    println!("Wrote {}", path.display());
    println!("It runs {} with the PATH of this shell.", program.display());
    println!("Start the Daemon with your login:");
    println!("  systemctl --user daemon-reload");
    println!("  systemctl --user enable --now {UNIT}");
    ExitCode::SUCCESS
}

pub fn uninstall() -> ExitCode {
    let Some(dir) = unit_dir() else {
        return failed(NO_CONFIG_HOME);
    };
    let enabled = dir.join("default.target.wants").join(UNIT);
    if std::fs::symlink_metadata(&enabled).is_ok() {
        if let Err(err) = std::fs::remove_file(&enabled) {
            return failed(format!(
                "{}: {err}; run `systemctl --user disable --now {UNIT}` first",
                enabled.display()
            ));
        }
        println!("Removed {}", enabled.display());
    }
    let path = dir.join(UNIT);
    match std::fs::remove_file(&path) {
        Ok(()) => println!("Removed {}", path.display()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            println!("{} was not installed", path.display());
        }
        Err(err) => return failed(format!("{}: {err}", path.display())),
    }
    println!("Stop a running unit and forget it:");
    println!("  systemctl --user disable --now {UNIT}");
    println!("  systemctl --user daemon-reload");
    ExitCode::SUCCESS
}

fn failed(message: impl std::fmt::Display) -> ExitCode {
    crate::client::fail("daemon", 1, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_path_stays_unquoted() {
        assert_eq!(exec_word("/usr/bin/orch"), "/usr/bin/orch");
    }

    #[test]
    fn a_path_with_whitespace_is_quoted() {
        assert_eq!(exec_word("/opt/my tools/orch"), "\"/opt/my tools/orch\"");
    }

    #[test]
    fn specifiers_and_variables_are_doubled() {
        assert_eq!(exec_word("/opt/100%/$HOME/orch"), "/opt/100%%/$$HOME/orch");
    }

    #[test]
    fn quotes_and_backslashes_are_escaped_inside_quotes() {
        assert_eq!(exec_word("/opt/a\"b\\c/orch"), "\"/opt/a\\\"b\\\\c/orch\"");
    }

    #[test]
    fn an_environment_assignment_is_quoted_and_escaped() {
        assert_eq!(
            environment_assignment("PATH", "/a b:/100%:\"x\"\\"),
            "\"PATH=/a b:/100%%:\\\"x\\\"\\\\\""
        );
    }
}
