mod gh;
mod git;
pub(crate) mod invocations;
pub(crate) mod paths;
pub(crate) mod shell;

use std::path::{Path, PathBuf};

use orch_core::GuardedAction;
use serde::{Deserialize, Serialize};

use crate::RuleVerdict;

use shell::{DirectoryChange, SimpleCommand, Word};

pub const GUARD_WAIT_SECS: u64 = 7 * 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuardKind {
    BaseBranch,
    OtherRef,
    WorktreeManagement,
    WriteOutsideWorktree,
    ExternalTool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardHit {
    pub kind: GuardKind,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardDecision {
    Allow,
    Ask(GuardHit),
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum GuardAnswer {
    Proceed,
    PresetAllow,
    Ask,
    Deny { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardOutcome {
    Answer(GuardAnswer),
    Prompt {
        hit: GuardHit,
        on_allow: GuardAnswer,
    },
}

pub fn guard_outcome(decision: GuardDecision, verdict: Option<RuleVerdict>) -> GuardOutcome {
    let on_allow = match verdict {
        Some(RuleVerdict::Deny { rule }) => {
            return GuardOutcome::Answer(GuardAnswer::Deny {
                reason: format!("The Preset rule {rule} denies this (Orchestrator)."),
            });
        }
        Some(RuleVerdict::Unverifiable) => return GuardOutcome::Answer(GuardAnswer::Ask),
        Some(RuleVerdict::Allow { .. }) => GuardAnswer::PresetAllow,
        None => GuardAnswer::Proceed,
    };
    match decision {
        GuardDecision::Allow => GuardOutcome::Answer(on_allow),
        GuardDecision::Ask(hit) => GuardOutcome::Prompt { hit, on_allow },
        GuardDecision::Unreadable => GuardOutcome::Answer(GuardAnswer::Ask),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GuardContext<'a> {
    pub worktree: &'a Path,
    pub branch: &'a str,
    pub base_branch: &'a str,
    pub enabled: bool,
    pub allowed: &'a [GuardHit],
    pub agent_dirs: &'a [PathBuf],
}

pub fn evaluate_guard(
    action: &GuardedAction,
    cwd: Option<&Path>,
    context: &GuardContext,
) -> GuardDecision {
    if !context.enabled {
        return GuardDecision::Allow;
    }
    let scope = GuardScope::new(context);
    let cwd = cwd.map_or_else(
        || scope.worktree.clone(),
        |cwd| paths::resolve(Path::new("/"), &cwd.to_string_lossy()),
    );
    let hits = match action {
        GuardedAction::WriteFile { path } => scope.write(&cwd, path).into_iter().collect(),
        GuardedAction::Shell { command } => scope.bash(&cwd, command),
        GuardedAction::ExternalTool { name } => vec![GuardHit {
            kind: GuardKind::ExternalTool,
            target: name.clone(),
        }],
        GuardedAction::Unreadable => return GuardDecision::Unreadable,
    };
    hits.into_iter()
        .find(|hit| !context.allowed.contains(hit))
        .map_or(GuardDecision::Allow, GuardDecision::Ask)
}

fn other_ref(target: &str) -> GuardHit {
    GuardHit {
        kind: GuardKind::OtherRef,
        target: target.into(),
    }
}

struct GuardScope<'a> {
    worktree: PathBuf,
    agent_dirs: Vec<PathBuf>,
    branch: &'a str,
    base_branch: &'a str,
}

impl<'a> GuardScope<'a> {
    fn new(context: &GuardContext<'a>) -> Self {
        Self {
            worktree: paths::resolve(Path::new("/"), &context.worktree.to_string_lossy()),
            agent_dirs: context
                .agent_dirs
                .iter()
                .map(|dir| paths::resolve(Path::new("/"), &dir.to_string_lossy()))
                .collect(),
            branch: context.branch,
            base_branch: context.base_branch,
        }
    }

    fn outside(&self, path: &Path) -> bool {
        !path.starts_with(&self.worktree)
            && !self.agent_dirs.iter().any(|dir| path.starts_with(dir))
            && !paths::is_harmless(path)
            && !paths::is_temp(path)
    }

    fn write(&self, cwd: &Path, raw: &str) -> Option<GuardHit> {
        let path = paths::resolve(cwd, raw);
        self.outside(&path).then(|| GuardHit {
            kind: GuardKind::WriteOutsideWorktree,
            target: path.to_string_lossy().into_owned(),
        })
    }

    fn written(&self, cwd: &Path, words: &[&Word]) -> Vec<GuardHit> {
        words
            .iter()
            .filter(|word| !word.expanded)
            .filter_map(|word| self.operand(cwd, word))
            .collect()
    }

    fn operand(&self, cwd: &Path, word: &Word) -> Option<GuardHit> {
        let Some(candidates) = word.candidates() else {
            return Some(GuardHit {
                kind: GuardKind::WriteOutsideWorktree,
                target: word.text.clone(),
            });
        };
        candidates
            .iter()
            .find_map(|candidate| self.write(cwd, &paths::widest(candidate)))
    }

    fn ref_hit(&self, name: &str) -> Option<GuardHit> {
        let short = name
            .strip_prefix("refs/heads/")
            .or_else(|| name.strip_prefix("heads/"))
            .unwrap_or(name);
        if short == self.branch || short == "HEAD" || short == "@" {
            None
        } else if short == self.base_branch {
            Some(GuardHit {
                kind: GuardKind::BaseBranch,
                target: short.into(),
            })
        } else {
            Some(other_ref(short))
        }
    }

    fn bash(&self, cwd: &Path, script: &str) -> Vec<GuardHit> {
        let mut cwd = cwd.to_path_buf();
        let mut cwds = vec![cwd.clone()];
        let mut lost = false;
        let mut hits = Vec::new();
        for command in shell::parse(script) {
            for target in &command.written {
                hits.extend(self.redirect(&cwds, lost, target));
            }
            for nested in &command.nested {
                hits.extend(self.bash(&cwd, nested));
            }
            hits.extend(self.simple_command(&cwd, &command));
            match command.directory_change() {
                Some(DirectoryChange::Into(dir)) => cwd = paths::resolve(&cwd, &dir.text),
                Some(DirectoryChange::Unknown) => lost = true,
                None => {}
            }
            if !cwds.contains(&cwd) {
                cwds.push(cwd.clone());
            }
        }
        hits
    }

    fn redirect(&self, cwds: &[PathBuf], lost: bool, target: &Word) -> Option<GuardHit> {
        let unknown = target.dynamic || lost && !Path::new(&target.text).is_absolute();
        if unknown {
            return Some(GuardHit {
                kind: GuardKind::WriteOutsideWorktree,
                target: target.text.clone(),
            });
        }
        cwds.iter().find_map(|cwd| self.write(cwd, &target.text))
    }

    fn simple_command(&self, cwd: &Path, command: &SimpleCommand) -> Vec<GuardHit> {
        let words = command.invocation();
        let Some((program, args)) = words.split_first() else {
            return Vec::new();
        };
        let operands: Vec<&Word> = args
            .iter()
            .copied()
            .filter(|word| !word.text.starts_with('-'))
            .collect();
        match program.text.rsplit('/').next().unwrap_or_default() {
            "touch" | "mkdir" | "rm" | "rmdir" | "tee" | "mv" | "truncate" => {
                self.written(cwd, &operands)
            }
            "cp" | "ln" | "install" => {
                self.written(cwd, &operands[operands.len().saturating_sub(1)..])
            }
            "git" => git::hits(self, cwd, args),
            "gh" => gh::hits(args).into_iter().collect(),
            _ => Vec::new(),
        }
    }
}
