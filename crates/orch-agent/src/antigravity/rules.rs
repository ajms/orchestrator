use std::path::{Component, Path, PathBuf};

use orch_core::GuardedAction;
use serde_json::Value;

use super::guards::MCP_PREFIX;
use crate::guard::paths;
use crate::guard::shell::{self, SimpleCommand, Word};
use crate::{RuleScope, RuleVerdict, Rules};

const ANY: &str = "*";
const WRAPPERS: [&str; 9] = [
    "env", "sudo", "doas", "command", "exec", "nohup", "time", "nice", "ionice",
];
const FLAGS_WITH_VALUES: [&str; 8] = ["-u", "-g", "-C", "-h", "-p", "-U", "-n", "-c"];
const KEYWORDS: [&str; 11] = [
    "if", "then", "else", "elif", "do", "while", "until", "!", "{", "}", "fi",
];
const SHELLS: [&str; 6] = ["sh", "bash", "zsh", "dash", "ksh", "fish"];

enum Rule<'a> {
    Command(Vec<&'a str>),
    WriteFile(&'a str),
    Mcp { server: &'a str, tool: &'a str },
    AnyMcp,
    Other,
}

impl<'a> Rule<'a> {
    fn parse(rule: &'a str) -> Self {
        let Some((kind, target)) = rule
            .trim()
            .strip_suffix(')')
            .and_then(|rule| rule.split_once('('))
        else {
            return Rule::Other;
        };
        let target = target.trim();
        match (kind.trim(), target.split_once('/')) {
            ("command", _) if target == ANY => Rule::Command(Vec::new()),
            ("command", _) => Rule::Command(target.split_whitespace().collect()),
            ("write_file", _) => Rule::WriteFile(target),
            ("mcp", _) if target == ANY => Rule::AnyMcp,
            ("mcp", Some((server, tool))) => Rule::Mcp { server, tool },
            _ => Rule::Other,
        }
    }
}

struct Parsed<'a> {
    text: &'a String,
    rule: Rule<'a>,
}

fn parse_all(rules: &[String]) -> Vec<Parsed<'_>> {
    rules
        .iter()
        .map(|text| Parsed {
            text,
            rule: Rule::parse(text),
        })
        .collect()
}

pub(super) fn verdict(
    rules: &Rules,
    action: &GuardedAction,
    scope: &RuleScope,
) -> Option<RuleVerdict> {
    let deny = parse_all(&rules.deny);
    let allow = parse_all(&rules.allow);
    let checker = Checker {
        scope,
        allow: &allow,
    };
    if let Some(rule) = deny.iter().find(|rule| checker.denies(action, &rule.rule)) {
        return Some(RuleVerdict::Deny {
            rule: rule.text.clone(),
        });
    }
    checker
        .allowed(action)
        .map(|rule| RuleVerdict::Allow { rule: rule.clone() })
}

struct Checker<'s, 'r> {
    scope: &'s RuleScope<'s>,
    allow: &'r [Parsed<'r>],
}

impl<'r> Checker<'_, 'r> {
    fn cwd(&self) -> PathBuf {
        let cwd = self.scope.cwd.unwrap_or(self.scope.worktree);
        paths::resolve(Path::new("/"), &cwd.to_string_lossy())
    }

    fn worktree(&self) -> PathBuf {
        paths::resolve(Path::new("/"), &self.scope.worktree.to_string_lossy())
    }

    fn denies(&self, action: &GuardedAction, rule: &Rule) -> bool {
        match (action, rule) {
            (GuardedAction::Shell { command }, Rule::Command(prefix)) => invocations(command)
                .iter()
                .any(|words| starts_with(words, prefix)),
            (GuardedAction::WriteFile { path }, Rule::WriteFile(pattern)) => {
                self.covers(&paths::resolve(&self.cwd(), path), pattern)
            }
            (GuardedAction::ExternalTool { name }, Rule::AnyMcp) => name.starts_with(MCP_PREFIX),
            (GuardedAction::ExternalTool { name }, Rule::Mcp { server, tool }) => {
                let prefix = format!("{MCP_PREFIX}{}_", underscored(server));
                match name.strip_prefix(&prefix) {
                    Some(_) if *tool == ANY => true,
                    Some(rest) => rest == underscored(tool),
                    None => false,
                }
            }
            _ => false,
        }
    }

    fn allowed(&self, action: &GuardedAction) -> Option<&'r String> {
        match action {
            GuardedAction::Shell { command } => self.allowed_line(command),
            GuardedAction::WriteFile { path } => {
                self.allowed_write(&paths::resolve(&self.cwd(), path))
            }
            GuardedAction::ExternalTool { name } => self.allowed_tool(name),
            GuardedAction::Unreadable => None,
        }
    }

    fn allowed_write(&self, path: &Path) -> Option<&'r String> {
        self.allow.iter().find_map(|parsed| match parsed.rule {
            Rule::WriteFile(pattern) => self.covers(path, pattern).then_some(parsed.text),
            _ => None,
        })
    }

    fn allowed_line(&self, line: &str) -> Option<&'r String> {
        let mut cwd = self.cwd();
        let mut first = None;
        for command in shell::parse(line) {
            let rule = self.allowed_command(&cwd, &command)?;
            first.get_or_insert(rule);
            if let [cd, dir] = command.words.as_slice()
                && cd.text == "cd"
            {
                cwd = paths::resolve(&cwd, &dir.text);
            }
        }
        first
    }

    fn allowed_command(&self, cwd: &Path, command: &SimpleCommand) -> Option<&'r String> {
        let words: Vec<&str> = command
            .words
            .iter()
            .map(|word| word.text.as_str())
            .collect();
        let plain = !words.is_empty()
            && command.nested.is_empty()
            && command.words.iter().all(|word| !word.dynamic)
            && !words[0].contains('=')
            && !WRAPPERS.contains(&words[0])
            && !KEYWORDS.contains(&words[0]);
        if !plain
            || !command
                .written
                .iter()
                .all(|word| self.allowed_target(cwd, word))
        {
            return None;
        }
        self.allow.iter().find_map(|parsed| match &parsed.rule {
            Rule::Command(prefix) => starts_with(&words, prefix).then_some(parsed.text),
            _ => None,
        })
    }

    fn allowed_target(&self, cwd: &Path, word: &Word) -> bool {
        if word.dynamic {
            return false;
        }
        let target = paths::resolve(cwd, &word.text);
        paths::is_harmless(&target) || self.allowed_write(&target).is_some()
    }

    fn allowed_tool(&self, name: &str) -> Option<&'r String> {
        let servers = mcp_servers(self.scope.lookup);
        self.allow.iter().find_map(|parsed| {
            let matches = match parsed.rule {
                Rule::AnyMcp => name.starts_with(MCP_PREFIX),
                Rule::Mcp { server, tool } => owner(name, &servers)
                    .filter(|(owner, _)| *owner == server)
                    .is_some_and(|(_, rest)| tool == ANY || rest == underscored(tool)),
                _ => false,
            };
            matches.then_some(parsed.text)
        })
    }

    fn covers(&self, path: &Path, pattern: &str) -> bool {
        if pattern == ANY {
            return true;
        }
        let pattern = paths::resolve(&self.worktree(), pattern);
        let pattern: Vec<String> = parts(&pattern);
        let path: Vec<String> = parts(path);
        let pattern: Vec<&str> = pattern.iter().map(String::as_str).collect();
        let path: Vec<&str> = path.iter().map(String::as_str).collect();
        glob_covers(&pattern, &path)
    }
}

fn parts(path: &Path) -> Vec<String> {
    path.components()
        .filter(|component| !matches!(component, Component::RootDir))
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect()
}

fn glob_covers(pattern: &[&str], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => true,
        Some((&"**", rest)) => (0..=path.len()).any(|skip| glob_covers(rest, &path[skip..])),
        Some((part, rest)) => path
            .split_first()
            .is_some_and(|(name, tail)| wildcard(part, name) && glob_covers(rest, tail)),
    }
}

fn wildcard(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == name,
        Some((head, tail)) => {
            let Some(rest) = name.strip_prefix(head) else {
                return false;
            };
            (0..=rest.len())
                .filter(|at| rest.is_char_boundary(*at))
                .any(|at| wildcard(tail, &rest[at..]))
        }
    }
}

fn starts_with<S: AsRef<str>>(words: &[S], prefix: &[&str]) -> bool {
    !words.is_empty()
        && words.len() >= prefix.len()
        && prefix.iter().zip(words).all(|(a, b)| *a == b.as_ref())
}

fn invocations(line: &str) -> Vec<Vec<String>> {
    let mut found = Vec::new();
    collect_invocations(line, &mut found, 0);
    found
}

fn collect_invocations(line: &str, found: &mut Vec<Vec<String>>, depth: usize) {
    if depth > 8 {
        return;
    }
    for command in shell::parse(line) {
        for nested in &command.nested {
            collect_invocations(nested, found, depth + 1);
        }
        let words = program_words(&command);
        if let Some(program) = words.first()
            && SHELLS.contains(&program.as_str())
            && let Some(script) = shell_script(&words[1..])
        {
            collect_invocations(script, found, depth + 1);
        }
        if !words.is_empty() {
            found.push(words);
        }
    }
}

fn program_words(command: &SimpleCommand) -> Vec<String> {
    let mut words = command
        .words
        .iter()
        .map(|word| word.text.as_str())
        .peekable();
    let mut wrapped = false;
    while let Some(word) = words.peek().copied() {
        if KEYWORDS.contains(&word) || word.contains('=') && !word.starts_with('-') {
            words.next();
        } else if WRAPPERS.contains(&word) {
            wrapped = true;
            words.next();
        } else if wrapped && word.starts_with('-') {
            words.next();
            if FLAGS_WITH_VALUES.contains(&word) {
                words.next();
            }
        } else {
            break;
        }
    }
    let mut words: Vec<String> = words.map(String::from).collect();
    if let Some(program) = words.first_mut() {
        *program = program.rsplit('/').next().unwrap_or_default().into();
    }
    words
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

fn underscored(name: &str) -> String {
    name.replace('-', "_")
}

fn owner<'s, 'n>(name: &'n str, servers: &'s [String]) -> Option<(&'s str, &'n str)> {
    let rest = name.strip_prefix(MCP_PREFIX)?;
    let mut owners = servers.iter().filter_map(|server| {
        rest.strip_prefix(&format!("{}_", underscored(server)))
            .map(|tool| (server.as_str(), tool))
    });
    let first = owners.next()?;
    owners.next().is_none().then_some(first)
}

fn mcp_servers(lookup: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let Some(home) = lookup("HOME") else {
        return Vec::new();
    };
    let config = Path::new(&home).join(".gemini/config");
    let plugins = std::fs::read_dir(config.join("plugins"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path().join("mcp_config.json"));
    std::iter::once(config.join("mcp_config.json"))
        .chain(plugins)
        .filter_map(|file| std::fs::read_to_string(file).ok())
        .filter_map(|text| serde_json::from_str::<Value>(&text).ok())
        .filter_map(|config| config.get("mcpServers")?.as_object().cloned())
        .flat_map(|servers| servers.keys().cloned().collect::<Vec<_>>())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}
