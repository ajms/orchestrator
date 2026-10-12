use std::io::BufRead;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use orch_agent::{AgentHookup, FileEdit, HookupError, apply_hookup, built_in_names, by_name};
use orch_config::xdg;

use crate::client;

#[derive(Subcommand)]
pub enum AgentCommand {
    Install(HookupArgs),
    Uninstall(HookupArgs),
}

#[derive(Args)]
pub struct HookupArgs {
    agent: String,
    #[arg(long)]
    yes: bool,
}

const DECLINED: u8 = 1;
const FAILED: u8 = 2;

#[derive(Clone, Copy)]
enum Change {
    Install,
    Uninstall,
}

impl Change {
    fn verb(self) -> &'static str {
        match self {
            Change::Install => "install",
            Change::Uninstall => "uninstall",
        }
    }

    fn plan(self, hookup: &dyn AgentHookup) -> Result<Vec<FileEdit>, HookupError> {
        match self {
            Change::Install => {
                let program = client::orch_program().map_err(|err| HookupError(err.to_string()))?;
                hookup.install(&program.to_string_lossy(), &xdg::process_env)
            }
            Change::Uninstall => hookup.uninstall(&xdg::process_env),
        }
    }
}

pub fn run(command: AgentCommand) -> ExitCode {
    let (change, args) = match command {
        AgentCommand::Install(args) => (Change::Install, args),
        AgentCommand::Uninstall(args) => (Change::Uninstall, args),
    };
    match hookup(change, &args) {
        Ok(code) => code,
        Err(err) => client::fail("agent", FAILED, err),
    }
}

fn hookup(change: Change, args: &HookupArgs) -> Result<ExitCode, String> {
    let adapter = by_name(&args.agent).ok_or_else(|| {
        let names = built_in_names().collect::<Vec<_>>().join(", ");
        format!(
            "unknown Agent \"{}\"; the built-in Agents are {names}",
            args.agent
        )
    })?;
    let Some(hookup) = adapter.hookup() else {
        println!(
            "The {} Agent needs no Agent hookup; orch configures it per launch.",
            args.agent
        );
        return Ok(ExitCode::SUCCESS);
    };
    let edits = change
        .plan(hookup.as_ref())
        .map_err(|err| err.to_string())?;
    let verb = change.verb();
    if edits.is_empty() {
        println!(
            "Nothing to {verb}: the {} Agent hookup is already {verb}ed.",
            args.agent
        );
        return Ok(ExitCode::SUCCESS);
    }
    for edit in &edits {
        print!("{}", render_diff(edit));
    }
    if !args.yes
        && !confirmed(&format!(
            "{} the {} Agent hookup?",
            capitalised(verb),
            args.agent
        ))
    {
        println!("Nothing changed.");
        return Ok(ExitCode::from(DECLINED));
    }
    apply_hookup(&edits).map_err(|err| err.to_string())?;
    println!("The {} Agent hookup is {verb}ed.", args.agent);
    Ok(ExitCode::SUCCESS)
}

fn capitalised(verb: &str) -> String {
    let mut chars = verb.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn confirmed(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let mut answer = String::new();
    let _ = std::io::stdin().lock().read_line(&mut answer);
    matches!(answer.trim(), "y" | "Y" | "yes")
}

fn render_diff(edit: &FileEdit) -> String {
    let path = edit.path.display();
    let mut out = match (&edit.before, &edit.after) {
        (None, _) => format!("--- /dev/null\n+++ {path}\n"),
        (_, None) => format!("--- {path}\n+++ /dev/null\n"),
        _ => format!("--- {path}\n+++ {path}\n"),
    };
    let before = lines(edit.before.as_deref());
    let after = lines(edit.after.as_deref());
    for (sign, line) in diff_lines(&before, &after) {
        out.push_str(&format!("{sign}{line}\n"));
    }
    out
}

fn lines(text: Option<&str>) -> Vec<&str> {
    text.map(|text| text.lines().collect()).unwrap_or_default()
}

fn diff_lines<'a>(before: &[&'a str], after: &[&'a str]) -> Vec<(char, &'a str)> {
    let mut common = vec![vec![0usize; after.len() + 1]; before.len() + 1];
    for i in (0..before.len()).rev() {
        for j in (0..after.len()).rev() {
            common[i][j] = match before[i] == after[j] {
                true => common[i + 1][j + 1] + 1,
                false => common[i + 1][j].max(common[i][j + 1]),
            };
        }
    }
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < before.len() || j < after.len() {
        if i < before.len() && j < after.len() && before[i] == after[j] {
            out.push((' ', before[i]));
            (i, j) = (i + 1, j + 1);
        } else if i < before.len() && (j == after.len() || common[i + 1][j] >= common[i][j + 1]) {
            out.push(('-', before[i]));
            i += 1;
        } else {
            out.push(('+', after[j]));
            j += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn diff(before: Option<&str>, after: Option<&str>) -> String {
        render_diff(&FileEdit {
            path: PathBuf::from("/h/settings.json"),
            before: before.map(Into::into),
            after: after.map(Into::into),
        })
    }

    #[test]
    fn the_diff_keeps_shared_lines_and_marks_the_changed_ones() {
        let shown = diff(
            Some("{\n  \"a\": 1,\n  \"b\": 2\n}\n"),
            Some("{\n  \"a\": 1,\n  \"b\": 3\n}\n"),
        );

        assert_eq!(
            shown,
            "--- /h/settings.json\n+++ /h/settings.json\n {\n   \"a\": 1,\n-  \"b\": 2\n+  \"b\": 3\n }\n"
        );
    }

    #[test]
    fn a_new_file_is_all_additions_and_a_removed_file_all_removals() {
        assert_eq!(
            diff(None, Some("{}\n")),
            "--- /dev/null\n+++ /h/settings.json\n+{}\n"
        );
        assert_eq!(
            diff(Some("{}\n"), None),
            "--- /h/settings.json\n+++ /dev/null\n-{}\n"
        );
    }
}
