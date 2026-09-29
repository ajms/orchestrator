mod gh;
mod git;
mod paths;
mod shell;

use std::path::{Path, PathBuf};

use serde_json::Value;

use shell::{SimpleCommand, Word};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuardKind {
    BaseBranch,
    OtherRef,
    WorktreeManagement,
    WriteOutsideWorktree,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardAnswer {
    Proceed,
    Ask,
    Deny { reason: String },
}

#[derive(Debug, Clone, Copy)]
pub struct GuardContext<'a> {
    pub worktree: &'a Path,
    pub branch: &'a str,
    pub base_branch: &'a str,
    pub enabled: bool,
    pub allowed: &'a [GuardHit],
}

pub fn evaluate_guard(
    tool: &str,
    input_json: &str,
    cwd: Option<&Path>,
    context: &GuardContext,
) -> GuardDecision {
    if !context.enabled {
        return GuardDecision::Allow;
    }
    let Ok(input) = serde_json::from_str::<Value>(input_json) else {
        return GuardDecision::Allow;
    };
    let scope = GuardScope::new(context);
    let cwd = cwd.map_or_else(
        || scope.worktree.clone(),
        |cwd| paths::resolve(Path::new("/"), &cwd.to_string_lossy()),
    );
    let field = |key: &str| input.get(key).and_then(Value::as_str);
    let hits = match tool {
        "Write" | "Edit" | "MultiEdit" => field("file_path")
            .and_then(|path| scope.write(&cwd, path))
            .into_iter()
            .collect(),
        "NotebookEdit" => field("notebook_path")
            .and_then(|path| scope.write(&cwd, path))
            .into_iter()
            .collect(),
        "Bash" => field("command")
            .map(|command| scope.bash(&cwd, command))
            .unwrap_or_default(),
        _ => Vec::new(),
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
    branch: &'a str,
    base_branch: &'a str,
}

impl<'a> GuardScope<'a> {
    fn new(context: &GuardContext<'a>) -> Self {
        Self {
            worktree: paths::resolve(Path::new("/"), &context.worktree.to_string_lossy()),
            branch: context.branch,
            base_branch: context.base_branch,
        }
    }

    fn outside(&self, path: &Path) -> bool {
        !path.starts_with(&self.worktree) && !paths::is_harmless(path)
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
            .filter(|word| !word.dynamic)
            .filter_map(|word| self.write(cwd, &word.text))
            .collect()
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
        let mut hits = Vec::new();
        for command in shell::parse(script) {
            let written: Vec<&Word> = command.written.iter().collect();
            hits.extend(self.written(&cwd, &written));
            hits.extend(self.simple_command(&mut cwd, &command));
        }
        hits
    }

    fn simple_command(&self, cwd: &mut PathBuf, command: &SimpleCommand) -> Vec<GuardHit> {
        const WRAPPERS: [&str; 6] = ["env", "sudo", "command", "exec", "nohup", "time"];
        let words: Vec<&Word> = command
            .words
            .iter()
            .skip_while(|word| word.text.contains('=') || WRAPPERS.contains(&word.text.as_str()))
            .collect();
        let Some((program, args)) = words.split_first() else {
            return Vec::new();
        };
        let operands: Vec<&Word> = args
            .iter()
            .copied()
            .filter(|word| !word.text.starts_with('-'))
            .collect();
        match program.text.rsplit('/').next().unwrap_or_default() {
            "cd" | "pushd" => {
                if let Some(dir) = operands.first().filter(|dir| !dir.dynamic) {
                    *cwd = paths::resolve(cwd, &dir.text);
                }
                Vec::new()
            }
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
