use crate::guard::shell;

pub(super) struct Wrapper {
    name: &'static str,
    value_flags: &'static str,
    positionals: usize,
}

const fn wrapper(name: &'static str, value_flags: &'static str, positionals: usize) -> Wrapper {
    Wrapper {
        name,
        value_flags,
        positionals,
    }
}

pub(super) const WRAPPERS: [Wrapper; 15] = [
    wrapper("env", "uCSP", 0),
    wrapper("sudo", "ugChpUrtDRT", 0),
    wrapper("doas", "uC", 0),
    wrapper("command", "", 0),
    wrapper("exec", "a", 0),
    wrapper("nohup", "", 0),
    wrapper("time", "fo", 0),
    wrapper("nice", "n", 0),
    wrapper("ionice", "cnp", 0),
    wrapper("timeout", "sk", 1),
    wrapper("stdbuf", "ioe", 0),
    wrapper("setsid", "", 0),
    wrapper("xargs", "IiLlnPsdEa", 0),
    wrapper("coproc", "", 0),
    wrapper("eval", "", 0),
];
pub(super) const KEYWORDS: [&str; 12] = [
    "if", "then", "else", "elif", "do", "while", "until", "!", "{", "}", "fi", "function",
];
const SHELLS: [&str; 6] = ["sh", "bash", "zsh", "dash", "ksh", "fish"];
const GIT_OPTIONS_WITH_VALUES: [&str; 6] = [
    "-C",
    "-c",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--config-env",
];
const MAX_DEPTH: usize = 8;

pub(super) fn is_wrapper(word: &str) -> bool {
    WRAPPERS.iter().any(|wrapper| wrapper.name == word)
}

pub(super) fn invocations(line: &str) -> Vec<Vec<String>> {
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
        let (words, scripts) = unwrap(&words);
        for script in scripts {
            collect(&script, found, depth + 1);
        }
        if let Some(program) = words.first()
            && SHELLS.contains(&program.as_str())
            && let Some(script) = shell_script(&words[1..])
        {
            collect(script, found, depth + 1);
        }
        if !words.is_empty() {
            found.push(words);
        }
    }
}

fn unwrap(words: &[&str]) -> (Vec<String>, Vec<String>) {
    let mut at = 0;
    let mut scripts = Vec::new();
    while let Some(&word) = words.get(at) {
        if word == "function" {
            at += 2;
        } else if KEYWORDS.contains(&word) || word.contains('=') && !word.starts_with('-') {
            at += 1;
        } else if word == "eval" {
            scripts.push(words[at + 1..].join(" "));
            return (Vec::new(), scripts);
        } else if let Some(wrapper) = WRAPPERS.iter().find(|wrapper| wrapper.name == word) {
            at = skip_flags(wrapper, words, at + 1, &mut scripts) + wrapper.positionals;
        } else {
            break;
        }
    }
    let mut words: Vec<String> = words
        .get(at..)
        .unwrap_or_default()
        .iter()
        .map(|word| word.to_string())
        .collect();
    if let Some(program) = words.first_mut() {
        *program = program.rsplit('/').next().unwrap_or_default().into();
    }
    if words.first().is_some_and(|program| program == "git") {
        let options = git_options(&words[1..]);
        words.drain(1..1 + options);
    }
    (words, scripts)
}

fn skip_flags(
    wrapper: &Wrapper,
    words: &[&str],
    mut at: usize,
    scripts: &mut Vec<String>,
) -> usize {
    while let Some(&flag) = words.get(at) {
        if flag == "--" {
            return at + 1;
        }
        if !flag.starts_with('-') || flag == "-" {
            break;
        }
        at += 1;
        if let Some(long) = flag.strip_prefix("--") {
            if let Some(script) = long.strip_prefix("split-string=") {
                scripts.push(script.into());
            }
            continue;
        }
        let letters = &flag[1..];
        let Some((pos, letter)) = letters
            .char_indices()
            .find(|(_, letter)| wrapper.value_flags.contains(*letter))
        else {
            continue;
        };
        let attached = &letters[pos + letter.len_utf8()..];
        let value = if attached.is_empty() {
            at += 1;
            words.get(at - 1).copied().unwrap_or_default()
        } else {
            attached
        };
        if wrapper.name == "env" && letter == 'S' {
            scripts.push(value.into());
        }
    }
    at
}

fn git_options(args: &[String]) -> usize {
    let mut at = 0;
    while let Some(option) = args.get(at) {
        if !option.starts_with('-') {
            break;
        }
        at += if GIT_OPTIONS_WITH_VALUES.contains(&option.as_str()) {
            2
        } else {
            1
        };
    }
    at.min(args.len())
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
