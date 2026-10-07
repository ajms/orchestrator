use orch_protocol::{CreateSession, Reply, Request, RequestError};
use serde_json::{Value, json};

use crate::cli::{orch, orch_with_input, stderr, stdout};
use crate::common::*;

const GUARD_WAIT_SECS: u64 = 7 * 24 * 60 * 60;

fn hooks_file(env: &Env) -> std::path::PathBuf {
    env.path("home/.gemini/config/hooks.json")
}

fn settings_file(env: &Env) -> std::path::PathBuf {
    env.path("home/.gemini/antigravity-cli/settings.json")
}

fn write_json(path: &std::path::Path, value: &Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}

fn read_json(path: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn users_hooks() -> Value {
    json!({
        "lint": {
            "PostToolUse": [
                { "matcher": "run_command", "hooks": [{ "command": "./lint.sh" }] }
            ]
        }
    })
}

fn users_settings() -> Value {
    json!({
        "trustedWorkspaces": ["/home/me/project"],
        "statusLine": { "type": "command", "command": "echo mine" }
    })
}

fn orch_command(subcommand: &str) -> String {
    format!(
        "'{}' {subcommand} --agent antigravity",
        env!("CARGO_BIN_EXE_orch")
    )
}

#[tokio::test]
async fn install_then_uninstall_restores_the_users_own_hooks_and_statusline() {
    let env = Env::new();
    write_json(&hooks_file(&env), &users_hooks());
    write_json(&settings_file(&env), &users_settings());

    let installed = orch(&env, &["agent", "install", "antigravity", "--yes"]).await;

    assert!(installed.status.success(), "{}", stderr(&installed));
    let hooks = read_json(&hooks_file(&env));
    assert_eq!(hooks["lint"], users_hooks()["lint"]);
    let hook = json!({ "type": "command", "command": orch_command("hook") });
    let mut guard_hook = hook.clone();
    guard_hook["timeout"] = json!(GUARD_WAIT_SECS);
    assert_eq!(
        hooks["orch"],
        json!({
            "PreToolUse": [{ "matcher": "*", "hooks": [guard_hook] }],
            "PostToolUse": [{ "matcher": "*", "hooks": [hook] }],
            "PreInvocation": [hook],
            "PostInvocation": [hook],
            "Stop": [hook],
        })
    );
    let settings = read_json(&settings_file(&env));
    assert_eq!(settings["statusLine"]["command"], orch_command("tap"));
    assert_eq!(settings["trustedWorkspaces"], json!(["/home/me/project"]));

    let uninstalled = orch(&env, &["agent", "uninstall", "antigravity", "--yes"]).await;

    assert!(uninstalled.status.success(), "{}", stderr(&uninstalled));
    assert_eq!(read_json(&hooks_file(&env)), users_hooks());
    assert_eq!(read_json(&settings_file(&env)), users_settings());
}

async fn orch_outside_a_session(env: &Env, args: &[&str], payload: &str) -> String {
    let output = orch_with_input(env, args, payload).await;
    assert!(output.status.success(), "{}", stderr(&output));
    stdout(&output)
}

const PRE_TOOL_USE: &str =
    r#"{"conversationId":"c1","toolCall":{"name":"run_command","args":{"CommandLine":"ls"}}}"#;
const STATUS: &str = r#"{"product":"antigravity","agent_state":"idle"}"#;

#[tokio::test]
async fn outside_orch_the_hook_leaves_the_decision_to_agy() {
    let env = Env::new();

    let answer =
        orch_outside_a_session(&env, &["hook", "--agent", "antigravity"], PRE_TOOL_USE).await;

    let answer: Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(answer, json!({ "decision": "ask" }));
}

#[tokio::test]
async fn outside_orch_the_tap_runs_only_the_users_saved_statusline() {
    let env = Env::new();
    let mut settings = users_settings();
    settings["statusLine"]["command"] = json!("printf 'mine '; cat");
    write_json(&settings_file(&env), &settings);
    orch(&env, &["agent", "install", "antigravity", "--yes"]).await;

    let line = orch_outside_a_session(&env, &["tap", "--agent", "antigravity"], STATUS).await;

    assert_eq!(line, format!("mine {STATUS}"));
}

#[tokio::test]
async fn without_a_saved_statusline_the_tap_prints_nothing_so_agys_own_line_shows() {
    let env = Env::new();
    orch(&env, &["agent", "install", "antigravity", "--yes"]).await;

    let line = orch_outside_a_session(&env, &["tap", "--agent", "antigravity"], STATUS).await;

    assert_eq!(line, "");
}

#[tokio::test]
async fn an_antigravity_session_is_refused_with_a_hint_until_the_hookup_is_installed() {
    let env = Env::new();
    let fake = format!(
        "[defaults.agents.antigravity]\nbinary = {:?}\nargs = [\"fake-agent\", \"--\"]\n",
        env!("CARGO_BIN_EXE_orch")
    );
    env.write_config(&fake);
    let repo = env.repo("app");
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let mut create = CreateSession::new(&repo, "Agy");
    create.agent = Some("antigravity".into());

    let refused = client.request(Request::CreateSession(create.clone())).await;

    assert!(
        matches!(&refused, Err(RequestError::Refused { message })
            if message.contains("orch agent install antigravity")),
        "{refused:?}"
    );
    assert!(client.session_list().await.is_empty());
    orch(&env, &["agent", "install", "antigravity", "--yes"]).await;
    let created = client.request(Request::CreateSession(create)).await;
    assert!(matches!(created, Ok(Reply::Created { .. })), "{created:?}");
}

#[tokio::test]
async fn doctor_reports_a_hookup_that_is_no_longer_in_place() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    orch(&env, &["agent", "install", "antigravity", "--yes"]).await;
    let healthy = orch(&env, &["doctor"]).await;
    assert!(healthy.status.success(), "{}", stdout(&healthy));
    write_json(&settings_file(&env), &users_settings());

    let output = orch(&env, &["doctor"]).await;

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let report = stdout(&output);
    assert!(report.contains("antigravity Agent hookup"), "{report}");
    assert!(report.contains("statusLine"), "{report}");
    assert!(
        report.contains("orch agent install antigravity"),
        "{report}"
    );
}

#[tokio::test]
async fn install_shows_the_diff_and_changes_nothing_without_a_yes() {
    let env = Env::new();
    write_json(&settings_file(&env), &users_settings());

    let declined = orch_with_input(&env, &["agent", "install", "antigravity"], "\n").await;

    assert_eq!(declined.status.code(), Some(1), "{}", stderr(&declined));
    let shown = stdout(&declined);
    assert!(
        shown.contains(&format!("+    \"command\": \"{}\"", orch_command("tap"))),
        "{shown}"
    );
    assert!(shown.contains("-    \"command\": \"echo mine\""), "{shown}");
    assert!(shown.contains(&orch_command("hook")), "{shown}");
    assert!(!hooks_file(&env).exists());
    assert_eq!(read_json(&settings_file(&env)), users_settings());
}

#[tokio::test]
async fn installing_again_changes_nothing_and_still_restores_the_users_statusline() {
    let env = Env::new();
    write_json(&settings_file(&env), &users_settings());
    orch(&env, &["agent", "install", "antigravity", "--yes"]).await;
    let first = std::fs::read_to_string(settings_file(&env)).unwrap();

    let again = orch(&env, &["agent", "install", "antigravity"]).await;

    assert!(again.status.success(), "{}", stderr(&again));
    assert_eq!(std::fs::read_to_string(settings_file(&env)).unwrap(), first);
    orch(&env, &["agent", "uninstall", "antigravity", "--yes"]).await;
    assert_eq!(read_json(&settings_file(&env)), users_settings());
}
