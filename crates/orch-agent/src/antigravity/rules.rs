use std::path::{Path, PathBuf};

use orch_core::GuardedAction;

use super::guards::MCP_PREFIX;
use crate::guard::paths;
use crate::guard::shell::{self, SimpleCommand};
use crate::{RuleVerdict, Rules};

const ANY: &str = "*";

enum Rule<'a> {
    Command(Vec<&'a str>),
    WriteFile(&'a str),
    Mcp(&'a str),
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
        match kind.trim() {
            "command" if target == ANY => Rule::Command(Vec::new()),
            "command" => Rule::Command(target.split_whitespace().collect()),
            "write_file" => Rule::WriteFile(target),
            "mcp" => Rule::Mcp(target),
            _ => Rule::Other,
        }
    }
}

enum Target<'a> {
    Commands(Vec<SimpleCommand>),
    Path { path: PathBuf, worktree: &'a Path },
    Tool(&'a str),
}

impl<'a> Target<'a> {
    fn of(action: &'a GuardedAction, cwd: Option<&Path>, worktree: &'a Path) -> Self {
        match action {
            GuardedAction::Shell { command } => Target::Commands(shell::parse(command)),
            GuardedAction::WriteFile { path } => Target::Path {
                path: paths::resolve(cwd.unwrap_or(worktree), path),
                worktree,
            },
            GuardedAction::ExternalTool { name } => Target::Tool(name),
        }
    }

    fn denied_by(&self, rule: &str) -> bool {
        match self {
            Target::Commands(commands) => commands.iter().any(|command| runs(command, rule)),
            _ => self.matches(rule),
        }
    }

    fn allowed_by<'r>(&self, rules: &'r [String]) -> Option<&'r String> {
        let matching = |test: &dyn Fn(&str) -> bool| rules.iter().find(|rule| test(rule));
        match self {
            Target::Commands(commands) => {
                let allowed = |command: &SimpleCommand| {
                    let fixed = command.words.iter().all(|word| !word.dynamic);
                    fixed
                        .then(|| matching(&|rule| runs(command, rule)))
                        .flatten()
                };
                let mut verdicts = commands.iter().map(allowed);
                let first = verdicts.next().flatten()?;
                verdicts.all(|verdict| verdict.is_some()).then_some(first)
            }
            _ => matching(&|rule| self.matches(rule)),
        }
    }

    fn matches(&self, rule: &str) -> bool {
        match (self, Rule::parse(rule)) {
            (Target::Path { path, worktree }, Rule::WriteFile(target)) => {
                target == ANY || path.starts_with(paths::resolve(worktree, target))
            }
            (Target::Tool(name), Rule::Mcp(target)) => mcp_matches(name, target),
            _ => false,
        }
    }
}

fn runs(command: &SimpleCommand, rule: &str) -> bool {
    let Rule::Command(prefix) = Rule::parse(rule) else {
        return false;
    };
    let words = command.invocation();
    !words.is_empty()
        && words.len() >= prefix.len()
        && prefix
            .iter()
            .zip(&words)
            .all(|(want, word)| *want == word.text)
}

fn mcp_matches(name: &str, target: &str) -> bool {
    let Some(tool) = name.strip_prefix(MCP_PREFIX) else {
        return false;
    };
    if target == ANY {
        return true;
    }
    let Some((server, wanted)) = target.split_once('/') else {
        return false;
    };
    let server = format!("{}_", server.replace('-', "_"));
    match tool.strip_prefix(&server) {
        Some(_) if wanted == ANY => true,
        Some(tool) => tool == wanted.replace('-', "_"),
        None => false,
    }
}

pub(super) fn verdict(
    rules: &Rules,
    action: &GuardedAction,
    cwd: Option<&Path>,
    worktree: &Path,
) -> Option<RuleVerdict> {
    let target = Target::of(action, cwd, worktree);
    if let Some(rule) = rules.deny.iter().find(|rule| target.denied_by(rule)) {
        return Some(RuleVerdict::Deny { rule: rule.clone() });
    }
    target
        .allowed_by(&rules.allow)
        .map(|rule| RuleVerdict::Allow { rule: rule.clone() })
}
