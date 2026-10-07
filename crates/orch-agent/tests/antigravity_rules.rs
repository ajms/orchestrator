use std::collections::BTreeMap;
use std::path::Path;

use orch_agent::{AgentAdapter, Antigravity, ClaudeCode, Preset, RuleVerdict, Rules};
use orch_core::GuardedAction;

const WORKTREE: &str = "/home/dev/shop/.orchestrator/worktrees/fix-login";

fn preset(agent: &str, allow: &[&str], deny: &[&str]) -> Preset {
    let rules = Rules {
        allow: allow.iter().map(|rule| rule.to_string()).collect(),
        deny: deny.iter().map(|rule| rule.to_string()).collect(),
    };
    Preset {
        name: "tight".into(),
        mode: None,
        rules: BTreeMap::from([(agent.to_string(), rules)]),
    }
}

fn verdict(preset: &Preset, action: GuardedAction) -> Option<RuleVerdict> {
    Antigravity::default().rule_verdict(preset, &action, None, Path::new(WORKTREE))
}

fn shell(command: &str) -> GuardedAction {
    GuardedAction::Shell {
        command: command.into(),
    }
}

fn write(path: &str) -> GuardedAction {
    GuardedAction::WriteFile { path: path.into() }
}

fn tool(name: &str) -> GuardedAction {
    GuardedAction::ExternalTool { name: name.into() }
}

fn allowed_by(rule: &str) -> Option<RuleVerdict> {
    Some(RuleVerdict::Allow { rule: rule.into() })
}

fn denied_by(rule: &str) -> Option<RuleVerdict> {
    Some(RuleVerdict::Deny { rule: rule.into() })
}

fn allows(rule: &str, action: GuardedAction) -> bool {
    verdict(&preset(Antigravity::NAME, &[rule], &[]), action) == allowed_by(rule)
}

#[test]
fn a_command_rule_matches_commands_that_start_with_its_words() {
    assert!(allows("command(git)", shell("git status")));
    assert!(allows("command(npm test)", shell("npm test -- --watch")));
    assert!(allows("command(*)", shell("make all")));
    assert!(!allows("command(git)", shell("gitk --all")));
    assert!(!allows("command(npm test)", shell("npm testing")));
    assert!(!allows("command(npm test)", shell("npm")));
}

#[test]
fn an_allowed_command_line_must_be_allowed_in_every_part() {
    assert!(allows("command(npm test)", shell("npm test && npm test")));
    assert!(!allows(
        "command(npm test)",
        shell("npm test && rm -rf src")
    ));
    assert!(!allows("command(echo)", shell("echo $(rm -rf src)")));
}

#[test]
fn a_denied_command_anywhere_in_the_line_denies_it() {
    let tight = preset(Antigravity::NAME, &["command(*)"], &["command(rm)"]);
    assert_eq!(
        verdict(&tight, shell("npm test && rm -rf src")),
        denied_by("command(rm)")
    );
    assert_eq!(
        verdict(&tight, shell("env rm -rf src")),
        denied_by("command(rm)")
    );
    assert_eq!(verdict(&tight, shell("npm test")), allowed_by("command(*)"));
}

#[test]
fn a_write_file_rule_covers_a_path_and_everything_under_it() {
    assert!(allows("write_file(*)", write("/etc/hosts")));
    assert!(allows(
        "write_file(/home/dev/shop)",
        write("/home/dev/shop/src/a.ts")
    ));
    assert!(allows(
        "write_file(/home/dev/shop)",
        write("/home/dev/shop")
    ));
    assert!(!allows(
        "write_file(/home/dev/shop)",
        write("/home/dev/shopping/a.ts")
    ));
}

#[test]
fn relative_write_file_rules_and_paths_are_in_the_worktree() {
    let target = format!("{WORKTREE}/.env");
    assert!(allows("write_file(.env)", write(&target)));
    assert!(allows("write_file(src)", write("src/login.ts")));
    assert!(!allows("write_file(.env)", write("/home/dev/.env")));
}

#[test]
fn a_mcp_rule_names_a_server_and_a_tool() {
    let snapshot = || tool("mcp_chrome_devtools_take_memory_snapshot");
    assert!(allows(
        "mcp(chrome-devtools/take_memory_snapshot)",
        snapshot()
    ));
    assert!(allows("mcp(chrome-devtools/*)", snapshot()));
    assert!(allows("mcp(*)", snapshot()));
    assert!(!allows("mcp(chrome-devtools/navigate_page)", snapshot()));
    assert!(!allows("mcp(github/*)", snapshot()));
    assert!(!allows("mcp(*)", tool("open_browser_url")));
}

#[test]
fn a_rule_only_matches_its_own_kind_of_action() {
    assert!(!allows("command(*)", write("/etc/hosts")));
    assert!(!allows("write_file(*)", shell("touch /etc/hosts")));
    assert!(!allows("read_url(github.com)", tool("open_browser_url")));
}

#[test]
fn agy_reads_only_its_own_rules() {
    let claudes = preset(ClaudeCode::NAME, &["Bash(git *)"], &["Bash(rm *)"]);
    assert_eq!(verdict(&claudes, shell("rm -rf src")), None);
    assert_eq!(
        ClaudeCode::default().rule_verdict(
            &preset(Antigravity::NAME, &[], &["command(rm)"]),
            &shell("rm -rf src"),
            None,
            Path::new(WORKTREE),
        ),
        None
    );
}
