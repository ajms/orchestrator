use std::path::{Component, Path, PathBuf};

use orch_core::GuardedAction;
use serde_json::Value;

use super::guards::MCP_PREFIX;
use crate::guard::invocations::{KEYWORDS, invocations, risky_git, runs_another_command};
use crate::guard::paths;
use crate::guard::shell::{self, DirectoryChange, SimpleCommand, Word};
use crate::{RuleScope, RuleVerdict, Rules};

const ANY: &str = "*";
const GIT_DIR: &str = ".git";

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
    if *action == GuardedAction::Unreadable {
        return (!rules.deny.is_empty()).then_some(RuleVerdict::Unverifiable);
    }
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

#[derive(Clone, Copy)]
enum Strictness {
    Deny,
    Allow,
}

impl Strictness {
    fn opens(self, name: &str) -> bool {
        matches!(self, Strictness::Deny) || !name.eq_ignore_ascii_case(GIT_DIR)
    }
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
            (GuardedAction::WriteFile { path }, Rule::WriteFile(pattern)) => self.covers(
                &paths::resolve(&self.cwd(), path),
                pattern,
                Strictness::Deny,
            ),
            (GuardedAction::ExternalTool { name }, Rule::AnyMcp) => name.starts_with(MCP_PREFIX),
            (GuardedAction::ExternalTool { name }, Rule::Mcp { server, tool }) => {
                match mcp_tool(name, server) {
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
            Rule::WriteFile(pattern) => self
                .covers(path, pattern, Strictness::Allow)
                .then_some(parsed.text),
            _ => None,
        })
    }

    fn allowed_line(&self, line: &str) -> Option<&'r String> {
        let mut cwd = self.cwd();
        let mut cwds = vec![cwd.clone()];
        let mut first = None;
        for command in shell::parse(line) {
            let plain_cd = matches!(command.words.as_slice(), [cd, _] if cd.text == "cd");
            let into = match command.directory_change() {
                Some(DirectoryChange::Into(dir)) if plain_cd => {
                    Some(paths::resolve(&cwd, &dir.text))
                }
                Some(_) => return None,
                None => None,
            };
            let rule = self.allowed_command(&cwds, &command)?;
            first.get_or_insert(rule);
            if let Some(into) = into {
                cwd = into;
                if !cwds.contains(&cwd) {
                    cwds.push(cwd.clone());
                }
            }
        }
        first
    }

    fn allowed_command(&self, cwds: &[PathBuf], command: &SimpleCommand) -> Option<&'r String> {
        let words: Vec<&str> = command
            .words
            .iter()
            .map(|word| word.text.as_str())
            .collect();
        let plain = !words.is_empty()
            && command.nested.is_empty()
            && command.words.iter().all(|word| !word.dynamic)
            && !words[0].contains('=')
            && !runs_another_command(words[0])
            && !KEYWORDS.contains(&words[0])
            && !(words[0] == "git" && risky_git(&words[1..]));
        if !plain
            || !command
                .written
                .iter()
                .all(|word| self.allowed_target(cwds, word))
        {
            return None;
        }
        self.allow.iter().find_map(|parsed| match &parsed.rule {
            Rule::Command(prefix) => starts_with(&words, prefix).then_some(parsed.text),
            _ => None,
        })
    }

    fn allowed_target(&self, cwds: &[PathBuf], word: &Word) -> bool {
        !word.dynamic
            && cwds.iter().all(|cwd| {
                let target = paths::resolve(cwd, &word.text);
                paths::is_harmless(&target) || self.allowed_write(&target).is_some()
            })
    }

    fn allowed_tool(&self, name: &str) -> Option<&'r String> {
        let servers = mcp_servers(self.scope.lookup);
        self.allow.iter().find_map(|parsed| {
            let matches = match parsed.rule {
                Rule::AnyMcp => name.starts_with(MCP_PREFIX),
                Rule::Mcp { server, tool } => owner(name, &servers)
                    .filter(|(owner, _)| *owner == server)
                    .is_some_and(|(_, rest)| {
                        !rest.contains('_') && (tool == ANY || rest == underscored(tool))
                    }),
                _ => false,
            };
            matches.then_some(parsed.text)
        })
    }

    fn covers(&self, path: &Path, pattern: &str, strictness: Strictness) -> bool {
        let path = parts(path);
        let path: Vec<&str> = path.iter().map(String::as_str).collect();
        if pattern == ANY {
            return path.iter().all(|name| strictness.opens(name));
        }
        let pattern = parts(&paths::resolve(&self.worktree(), pattern));
        let pattern: Vec<&str> = pattern.iter().map(String::as_str).collect();
        glob_covers(&pattern, &path, strictness)
    }
}

fn parts(path: &Path) -> Vec<String> {
    path.components()
        .filter(|component| !matches!(component, Component::RootDir))
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect()
}

fn glob_covers(pattern: &[&str], path: &[&str], strictness: Strictness) -> bool {
    let open = |name: &&str| strictness.opens(name);
    match pattern.split_first() {
        None => path.iter().all(open),
        Some((&"**", rest)) => (0..=path.len())
            .take_while(|skip| path[..*skip].iter().all(open))
            .any(|skip| glob_covers(rest, &path[skip..], strictness)),
        Some((part, rest)) => path.split_first().is_some_and(|(name, tail)| {
            let named = part == name || part.contains('*') && open(name);
            named && wildcard(part, name) && glob_covers(rest, tail, strictness)
        }),
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

fn underscored(name: &str) -> String {
    name.replace('-', "_")
}

fn mcp_tool<'n>(name: &'n str, server: &str) -> Option<&'n str> {
    name.strip_prefix(MCP_PREFIX)?
        .strip_prefix(&format!("{}_", underscored(server)))
}

fn owner<'s, 'n>(name: &'n str, servers: &'s [String]) -> Option<(&'s str, &'n str)> {
    let mut owners = servers
        .iter()
        .filter_map(|server| mcp_tool(name, server).map(|tool| (server.as_str(), tool)));
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
