use std::collections::BTreeMap;
use std::path::Path;

use orch_agent::{AgentAdapter, Antigravity, ClaudeCode, Preset, RuleScope, RuleVerdict, Rules};
use orch_core::GuardedAction;
use serde_json::json;

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

fn no_env(_: &str) -> Option<String> {
    None
}

fn verdict_in(
    preset: &Preset,
    action: GuardedAction,
    worktree: &Path,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Option<RuleVerdict> {
    let scope = RuleScope {
        cwd: None,
        worktree,
        lookup,
    };
    Antigravity::default().rule_verdict(preset, &action, &scope)
}

fn verdict(preset: &Preset, action: GuardedAction) -> Option<RuleVerdict> {
    verdict_in(preset, action, Path::new(WORKTREE), &no_env)
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

fn denies(rule: &str, action: GuardedAction) -> bool {
    verdict(&preset(Antigravity::NAME, &[], &[rule]), action) == denied_by(rule)
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
fn an_allow_rule_never_covers_a_wrapper_or_an_assignment() {
    for line in [
        "sudo git push",
        "env PATH=/tmp/x git status",
        "LD_PRELOAD=./e.so git status",
        "GIT_SSH_COMMAND='rm -rf ~' git fetch",
        "nohup git gc",
    ] {
        assert!(!allows("command(git)", shell(line)), "{line}");
        assert!(!allows("command(*)", shell(line)), "{line}");
    }
}

#[test]
fn an_allowed_command_that_redirects_into_a_file_needs_that_write_allowed_too() {
    assert!(!allows("command(echo)", shell("echo hi > /etc/passwd")));
    assert!(allows("command(echo)", shell("echo hi > /dev/null 2>&1")));
    let both = preset(
        Antigravity::NAME,
        &["command(echo)", "write_file(notes)"],
        &[],
    );
    assert_eq!(
        verdict(&both, shell("echo hi >> notes/today.md")),
        allowed_by("command(echo)")
    );
    assert_eq!(verdict(&both, shell("echo hi > /etc/passwd")), None);
}

#[test]
fn a_denied_command_anywhere_in_the_line_denies_it() {
    let tight = preset(Antigravity::NAME, &["command(*)"], &["command(rm)"]);
    assert_eq!(
        verdict(&tight, shell("npm test && rm -rf src")),
        denied_by("command(rm)")
    );
    assert_eq!(verdict(&tight, shell("npm test")), allowed_by("command(*)"));
}

#[test]
fn a_deny_rule_sees_through_wrappers_paths_keywords_and_nested_shells() {
    for line in [
        "env rm -rf src",
        "FOO=1 rm -rf src",
        "sudo -u root rm -rf src",
        "/bin/rm -rf src",
        "if true; then rm -rf src; fi",
        "echo $(rm -rf src)",
        "echo `rm -rf src`",
        "sh -c 'rm -rf src'",
        "bash -lc \"cd /tmp && rm -rf src\"",
    ] {
        assert!(denies("command(rm)", shell(line)), "{line}");
    }
    assert!(!denies("command(rm)", shell("echo rm")));
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
fn write_file_rules_take_simple_globs() {
    assert!(allows("write_file(src/*.ts)", write("src/login.ts")));
    assert!(!allows("write_file(src/*.ts)", write("src/login.rs")));
    assert!(!allows("write_file(src/*.ts)", write("lib/login.ts")));
    assert!(allows("write_file(**/.env)", write(".env")));
    assert!(allows("write_file(**/.env)", write("config/local/.env")));
    assert!(denies("write_file(**/.env)", write("config/.env")));
    assert!(!denies("write_file(**/.env)", write("config/.envrc")));
}

#[test]
fn a_write_file_rule_cannot_be_escaped_through_a_symlink_and_dotdot() {
    let root = tempfile::tempdir().unwrap();
    let worktree = root.path().join("wt");
    let elsewhere = root.path().join("elsewhere/deep");
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, worktree.join("link")).unwrap();
    let rule = format!("write_file({})", worktree.display());
    let tight = preset(Antigravity::NAME, &[&rule], &[]);

    let escape = format!("{}/link/../secret", worktree.display());
    assert_eq!(verdict_in(&tight, write(&escape), &worktree, &no_env), None);
    let inside = format!("{}/src/../a.ts", worktree.display());
    assert_eq!(
        verdict_in(&tight, write(&inside), &worktree, &no_env),
        allowed_by(&rule)
    );
}

struct AgyHome(tempfile::TempDir);

impl AgyHome {
    fn with_servers(servers: &[&str]) -> Self {
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join(".gemini/config");
        std::fs::create_dir_all(&config).unwrap();
        let servers: serde_json::Map<_, _> = servers
            .iter()
            .map(|name| (name.to_string(), json!({ "command": "true" })))
            .collect();
        std::fs::write(
            config.join("mcp_config.json"),
            json!({ "mcpServers": servers }).to_string(),
        )
        .unwrap();
        Self(home)
    }

    fn with_plugin_server(self, plugin: &str, server: &str) -> Self {
        let dir = self.0.path().join(".gemini/config/plugins").join(plugin);
        std::fs::create_dir_all(&dir).unwrap();
        let config = json!({ "mcpServers": { server: { "url": "https://example.com/mcp" } } });
        std::fs::write(dir.join("mcp_config.json"), config.to_string()).unwrap();
        self
    }

    fn allows(&self, rule: &str, name: &str) -> bool {
        let home = self.0.path().to_string_lossy().into_owned();
        let lookup = move |key: &str| (key == "HOME").then(|| home.clone());
        let tight = preset(Antigravity::NAME, &[rule], &[]);
        verdict_in(&tight, tool(name), Path::new(WORKTREE), &lookup) == allowed_by(rule)
    }
}

#[test]
fn a_mcp_rule_names_a_server_and_a_tool() {
    let home = AgyHome::with_servers(&["chrome-devtools", "github"]);
    let snapshot = "mcp_chrome_devtools_take_memory_snapshot";
    assert!(home.allows("mcp(chrome-devtools/take_memory_snapshot)", snapshot));
    assert!(home.allows("mcp(chrome-devtools/*)", snapshot));
    assert!(home.allows("mcp(*)", snapshot));
    assert!(!home.allows("mcp(chrome-devtools/navigate_page)", snapshot));
    assert!(!home.allows("mcp(github/*)", snapshot));
    assert!(!home.allows("mcp(*)", "open_browser_url"));
}

#[test]
fn a_mcp_server_rule_covers_only_that_exact_server() {
    let home = AgyHome::with_servers(&["chrome", "chrome-devtools"]);
    let snapshot = "mcp_chrome_devtools_take_memory_snapshot";
    assert!(!home.allows("mcp(chrome/*)", snapshot));
    assert!(!home.allows("mcp(chrome/devtools_take_memory_snapshot)", snapshot));
    assert!(home.allows("mcp(chrome/open_tab)", "mcp_chrome_open_tab"));
}

#[test]
fn servers_whose_names_collide_once_dashes_become_underscores_are_never_allowed() {
    let home = AgyHome::with_servers(&["my-db", "my_db"]);
    assert!(!home.allows("mcp(my-db/*)", "mcp_my_db_query"));
    assert!(!home.allows("mcp(my_db/query)", "mcp_my_db_query"));
}

#[test]
fn a_mcp_tool_from_a_server_agy_does_not_list_is_never_allowed() {
    assert!(!allows(
        "mcp(chrome-devtools/*)",
        tool("mcp_chrome_devtools_x")
    ));
    let home = AgyHome::with_servers(&["github"]);
    assert!(!home.allows("mcp(chrome-devtools/*)", "mcp_chrome_devtools_x"));
    let home = home.with_plugin_server("devtools", "chrome-devtools");
    assert!(home.allows("mcp(chrome-devtools/*)", "mcp_chrome_devtools_x"));
}

#[test]
fn a_mcp_deny_rule_matches_without_knowing_the_servers() {
    assert!(denies(
        "mcp(chrome-devtools/*)",
        tool("mcp_chrome_devtools_x")
    ));
    assert!(denies(
        "mcp(chrome-devtools/x)",
        tool("mcp_chrome_devtools_x")
    ));
}

#[test]
fn a_rule_only_matches_its_own_kind_of_action() {
    assert!(!allows("command(*)", write("/etc/hosts")));
    assert!(!allows("write_file(*)", shell("touch /etc/hosts")));
    assert!(!allows("read_url(github.com)", tool("open_browser_url")));
}

#[test]
fn a_tool_call_orch_could_not_read_is_never_allowed_or_denied_by_a_rule() {
    let everything = preset(
        Antigravity::NAME,
        &["command(*)", "write_file(*)", "mcp(*)"],
        &["command(*)", "write_file(*)", "mcp(*)"],
    );
    assert_eq!(verdict(&everything, GuardedAction::Unreadable), None);
}

#[test]
fn agy_reads_only_its_own_rules() {
    let claudes = preset(ClaudeCode::NAME, &["Bash(git *)"], &["Bash(rm *)"]);
    assert_eq!(verdict(&claudes, shell("rm -rf src")), None);
    let scope = RuleScope {
        cwd: None,
        worktree: Path::new(WORKTREE),
        lookup: &no_env,
    };
    assert_eq!(
        ClaudeCode::default().rule_verdict(
            &preset(Antigravity::NAME, &[], &["command(rm)"]),
            &shell("rm -rf src"),
            &scope,
        ),
        None
    );
}
