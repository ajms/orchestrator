use super::shell;

struct Wrapper {
    name: &'static str,
    value_flags: &'static str,
    long_values: &'static [&'static str],
    positionals: usize,
    script_flag: Option<(char, &'static str)>,
}

const fn wrapper(
    name: &'static str,
    value_flags: &'static str,
    long_values: &'static [&'static str],
) -> Wrapper {
    Wrapper {
        name,
        value_flags,
        long_values,
        positionals: 0,
        script_flag: None,
    }
}

const WRAPPERS: [Wrapper; 14] = [
    Wrapper {
        script_flag: Some(('S', "--split-string")),
        ..wrapper("env", "uCSP", &["--unset", "--chdir", "--split-string"])
    },
    wrapper(
        "sudo",
        "ugChpUrtDRT",
        &[
            "--user",
            "--group",
            "--chdir",
            "--host",
            "--prompt",
            "--other-user",
            "--role",
            "--type",
            "--close-from",
            "--command-timeout",
        ],
    ),
    wrapper("doas", "uC", &[]),
    wrapper("command", "", &[]),
    wrapper("exec", "a", &[]),
    wrapper("nohup", "", &[]),
    wrapper("time", "fo", &["--format", "--output"]),
    wrapper("nice", "n", &["--adjustment"]),
    wrapper("ionice", "cnp", &["--class", "--classdata", "--pid"]),
    Wrapper {
        positionals: 1,
        ..wrapper("timeout", "sk", &["--signal", "--kill-after"])
    },
    wrapper("stdbuf", "ioe", &["--input", "--output", "--error"]),
    wrapper("setsid", "", &[]),
    wrapper(
        "xargs",
        "IiLlnPsdEa",
        &[
            "--max-args",
            "--max-lines",
            "--max-procs",
            "--max-chars",
            "--delimiter",
            "--eof",
            "--arg-file",
            "--process-slot-var",
        ],
    ),
    wrapper("coproc", "", &[]),
];
const EVAL: &str = "eval";
pub(crate) const KEYWORDS: [&str; 12] = [
    "if", "then", "else", "elif", "do", "while", "until", "!", "{", "}", "fi", "function",
];
const SHELLS: [&str; 6] = ["sh", "bash", "zsh", "dash", "ksh", "fish"];
const FIND_EXECS: [&str; 4] = ["-exec", "-execdir", "-ok", "-okdir"];
const MAX_DEPTH: usize = 8;

pub(super) const GIT_GLOBALS_WITH_VALUES: [&str; 6] = [
    "-C",
    "-c",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--config-env",
];
const RISKY_GIT_GLOBALS: [&str; 3] = ["-c", "--config-env", "--exec-path"];
const RISKY_GIT_SUBCOMMANDS: [&str; 3] = ["config", "filter-branch", "bundle"];
const REMOTE_GIT_SUBCOMMANDS: [&str; 5] = ["clone", "fetch", "pull", "ls-remote", "push"];

pub(crate) fn runs_another_command(word: &str) -> bool {
    word == EVAL || WRAPPERS.iter().any(|wrapper| wrapper.name == word)
}

pub(crate) fn invocations(line: &str) -> Vec<Vec<String>> {
    let mut found = Vec::new();
    collect(line, &mut found, 0);
    found
}

fn collect(line: &str, found: &mut Vec<Vec<String>>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for command in shell::parse(line) {
        for nested in &command.nested {
            collect(nested, found, depth + 1);
        }
        let words: Vec<&str> = command
            .words
            .iter()
            .map(|word| word.text.as_str())
            .collect();
        collect_words(&words, found, depth);
    }
}

fn collect_words(words: &[&str], found: &mut Vec<Vec<String>>, depth: usize) {
    let (words, scripts) = program_words(words);
    for script in scripts {
        collect(&script, found, depth + 1);
    }
    match words.first().map(String::as_str) {
        Some(program) if SHELLS.contains(&program) => {
            if let Some(script) = shell_script(&words[1..]) {
                collect(script, found, depth + 1);
            }
        }
        Some("find") => {
            for executed in find_executions(&words[1..]) {
                collect_words(&executed, found, depth + 1);
            }
        }
        _ => {}
    }
    if !words.is_empty() {
        found.push(words);
    }
}

pub(super) fn program_start<S: AsRef<str>>(words: &[S], scripts: &mut Vec<String>) -> usize {
    let mut at = 0;
    while let Some(word) = words.get(at).map(AsRef::as_ref) {
        if word == "function" {
            at += 2;
        } else if KEYWORDS.contains(&word) || word.contains('=') && !word.starts_with('-') {
            at += 1;
        } else if let Some(wrapper) = WRAPPERS.iter().find(|wrapper| wrapper.name == word) {
            at = skip_flags(wrapper, words, at + 1, scripts) + wrapper.positionals;
        } else {
            break;
        }
    }
    at.min(words.len())
}

fn program_words(words: &[&str]) -> (Vec<String>, Vec<String>) {
    let mut scripts = Vec::new();
    let at = program_start(words, &mut scripts);
    if words.get(at) == Some(&EVAL) {
        scripts.push(words[at + 1..].join(" "));
        return (Vec::new(), scripts);
    }
    let mut words: Vec<String> = words[at..].iter().map(|word| word.to_string()).collect();
    if let Some(program) = words.first_mut() {
        *program = program.rsplit('/').next().unwrap_or_default().into();
    }
    if words.first().is_some_and(|program| program == "git") {
        let globals = git_globals(&words[1..]);
        words.drain(1..1 + globals);
    }
    (words, scripts)
}

fn skip_flags<S: AsRef<str>>(
    wrapper: &Wrapper,
    words: &[S],
    mut at: usize,
    scripts: &mut Vec<String>,
) -> usize {
    let value = |at: &mut usize| {
        *at += 1;
        words.get(*at - 1).map_or("", AsRef::as_ref)
    };
    while let Some(flag) = words.get(at).map(AsRef::as_ref) {
        if flag == "--" {
            return at + 1;
        }
        if !flag.starts_with('-') || flag == "-" {
            break;
        }
        at += 1;
        let (taken, is_script) = if flag.starts_with("--") {
            let (name, attached) = match flag.split_once('=') {
                Some((name, attached)) => (name, Some(attached)),
                None => (flag, None),
            };
            let is_script = wrapper.script_flag.is_some_and(|(_, long)| long == name);
            match attached {
                Some(attached) => (Some(attached), is_script),
                None if wrapper.long_values.contains(&name) => (Some(value(&mut at)), is_script),
                None => (None, false),
            }
        } else {
            let letters = &flag[1..];
            match letters
                .char_indices()
                .find(|(_, letter)| wrapper.value_flags.contains(*letter))
            {
                Some((pos, letter)) => {
                    let attached = &letters[pos + letter.len_utf8()..];
                    let taken = if attached.is_empty() {
                        value(&mut at)
                    } else {
                        attached
                    };
                    let is_script = wrapper
                        .script_flag
                        .is_some_and(|(short, _)| short == letter);
                    (Some(taken), is_script)
                }
                None => (None, false),
            }
        };
        if let Some(script) = taken.filter(|_| is_script) {
            scripts.push(script.into());
        }
    }
    at
}

fn find_executions(args: &[String]) -> Vec<Vec<&str>> {
    let mut executions = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if FIND_EXECS.contains(&arg.as_str()) {
            let executed = args
                .by_ref()
                .map(String::as_str)
                .take_while(|arg| *arg != ";" && *arg != "+")
                .collect();
            executions.push(executed);
        }
    }
    executions
}

fn git_globals<S: AsRef<str>>(args: &[S]) -> usize {
    let mut at = 0;
    while let Some(option) = args.get(at).map(AsRef::as_ref) {
        if !option.starts_with('-') {
            break;
        }
        at += if GIT_GLOBALS_WITH_VALUES.contains(&option) {
            2
        } else {
            1
        };
    }
    at.min(args.len())
}

pub(crate) fn risky_git(args: &[&str]) -> bool {
    let globals = git_globals(args);
    let names = |word: &str| word.split('=').next().unwrap_or_default().to_string();
    if args[..globals]
        .iter()
        .any(|option| RISKY_GIT_GLOBALS.contains(&names(option).as_str()))
    {
        return true;
    }
    let Some((&subcommand, rest)) = args[globals..].split_first() else {
        return false;
    };
    let has = |short: char, long: &[&str]| {
        rest.iter().any(|arg| {
            let long_match = long.iter().any(|name| names(arg) == *name);
            let short_match =
                !arg.starts_with("--") && arg.starts_with('-') && arg[1..].contains(short);
            long_match || short_match
        })
    };
    rest.iter().any(|arg| arg.starts_with("--output"))
        || RISKY_GIT_SUBCOMMANDS.contains(&subcommand)
        || match subcommand {
            "rebase" => has('x', &["--exec"]),
            "difftool" => has('x', &["--extcmd"]),
            "submodule" => rest.first() == Some(&"foreach"),
            "archive" | "format-patch" => has('o', &["--output"]),
            _ if REMOTE_GIT_SUBCOMMANDS.contains(&subcommand) => {
                has('u', &["--upload-pack", "--receive-pack", "--exec"])
            }
            _ => false,
        }
}

fn shell_script(args: &[String]) -> Option<&str> {
    let at = args
        .iter()
        .position(|arg| arg.starts_with('-') && !arg.starts_with("--") && arg.contains('c'))?;
    args[at + 1..]
        .iter()
        .find(|arg| !arg.starts_with('-'))
        .map(String::as_str)
}
