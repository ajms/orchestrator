use std::path::PathBuf;
use std::time::Duration;

use orch_core::TranscriptEntry;
use orch_protocol::{
    AgentStateView as State, CreateSession, PhaseView, Reply, Request, UsageTotalsView,
};

use crate::bwrap::bwrap_runs;
use crate::common::*;

const PI: &str = "[defaults.agent]\nname = \"pi\"\n";

fn pi_env() -> Env {
    let env = Env::new();
    let orch = env!("CARGO_BIN_EXE_orch");
    env.write_config(&format!(
        "{PI}[defaults.agents.pi]\nbinary = {orch:?}\nargs = [\"fake-agent\", \"--\"]\n"
    ));
    let user = env.path("home/.pi/agent");
    std::fs::create_dir_all(user.join("sessions")).unwrap();
    std::fs::write(user.join("settings.json"), r#"{"theme":"dark"}"#).unwrap();
    env
}

#[tokio::test]
async fn a_pi_session_runs_pi_from_its_agent_profile() {
    let env = pi_env();
    let settings = env.path("home/.pi/agent/settings.json");
    let modified = settings.metadata().unwrap().modified().unwrap();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let repo = env.repo("app");

    let id = client.create(CreateSession::new(&repo, "Fix it")).await;

    let view = client
        .until(&id, "running", |view| view.agent_state.is_some())
        .await;
    assert_eq!(view.agent, "pi");
    assert_eq!(view.agent_state, Some(State::Starting));
    assert_eq!(view.conversation.as_deref(), Some(id.as_str()));
    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text("arg> -na").await;
    pane.wait_for_text(&format!("session-id> {}", id.as_str()))
        .await;
    pane.wait_for_text("prompt> Fix it").await;

    let profile = env.path("state/orchestrator/agent-profiles/pi");
    let extension = profile.join("extensions/orch/extension/index.ts");
    pane.wait_for_text(&format!("arg> {}", extension.display()))
        .await;
    pane.wait_for_text(&format!("orch-bin> {}", env!("CARGO_BIN_EXE_orch")))
        .await;
    pane.wait_for_text(&format!("orch-session> {}", id.as_str()))
        .await;
    assert!(extension.is_file());
    pane.type_line("env PI_CODING_AGENT_DIR").await;
    pane.wait_for_text(&format!("PI_CODING_AGENT_DIR={}", profile.display()))
        .await;
    pane.type_line("env PI_OFFLINE").await;
    pane.wait_for_text("PI_OFFLINE=1").await;
    let generated = std::fs::read_to_string(profile.join("settings.json")).unwrap();
    assert!(generated.contains("\"theme\": \"dark\""), "{generated}");
    assert_eq!(settings.metadata().unwrap().modified().unwrap(), modified);
}

#[tokio::test]
async fn a_pi_session_resumes_the_same_conversation_after_a_reboot() {
    let env = pi_env();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let repo = env.repo("app");
    let id = client.create(CreateSession::new(&repo, "Fix it")).await;
    client
        .until(&id, "held", |view| view.holder_pid.is_some())
        .await;
    let holder = client.holder_pid(&id).unwrap();

    daemon.kill();
    kill(holder, "-KILL");
    wait_until("the Holder to die", || process_gone(holder)).await;
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    client
        .until(&id, "Suspended", |view| view.phase == PhaseView::Suspended)
        .await;

    let resumed = client
        .request(Request::Resume {
            session: id.clone(),
        })
        .await;
    assert_eq!(resumed, Ok(Reply::Done));
    client
        .until(&id, "running again", |view| view.agent_state.is_some())
        .await;
    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text(&format!("session-id> {}", id.as_str()))
        .await;
    pane.type_line("print announced").await;
    pane.wait_for_text("announced").await;
    assert!(!pane.text().contains("prompt>"));
}

#[tokio::test]
async fn sandboxed_pi_sees_its_profile_but_not_the_users_sessions() {
    if !bwrap_runs() {
        return;
    }
    let env = pi_env();
    env.real_bwrap();
    let own = env.path("home/.pi/agent/sessions/--home--/own.jsonl");
    std::fs::create_dir_all(own.parent().unwrap()).unwrap();
    std::fs::write(&own, "mine").unwrap();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let repo = env.repo("app");
    let id = client.create(CreateSession::new(&repo, "Boxed")).await;
    client
        .until(&id, "running", |view| view.agent_state.is_some())
        .await;
    let mut pane = env.pane(&id, PANE).await;
    let profile = env.path("state/orchestrator/agent-profiles/pi");

    for (command, path, outcome) in [
        ("exists", own.clone(), "no"),
        ("exists", env.path("home/.pi/agent/settings.json"), "yes"),
        ("write", env.path("home/.pi/agent/settings.json"), "denied"),
        ("write", profile.join("trust.json"), "ok"),
        ("write", profile.join("settings.json"), "ok"),
        ("write", profile.join("extensions/planted.ts"), "denied"),
    ] {
        let line = format!("{command} {}", path.display());
        pane.type_line(&line).await;
        let answer = format!("{line} -> ");
        pane.wait_for(&answer, |pane| pane.text().contains(&answer))
            .await;
        let text = pane.text();
        let got = text[text.find(&answer).unwrap() + answer.len()..]
            .lines()
            .next()
            .unwrap()
            .trim()
            .to_string();
        match outcome {
            "denied" => assert_ne!(got, "ok", "{line}"),
            _ => assert_eq!(got, outcome, "{line}"),
        }
    }
}

async fn resumed_pane(env: &Env, client: &mut TestClient, id: &orch_core::SessionId) -> PaneView {
    let resumed = client
        .request(Request::Resume {
            session: id.clone(),
        })
        .await;
    assert_eq!(resumed, Ok(Reply::Done));
    client
        .until(id, "running again", |view| {
            view.phase == PhaseView::Active && view.holder_pid.is_some()
        })
        .await;
    let mut pane = env.pane(id, PANE).await;
    pane.wait_for_text(&format!("session-id> {}", id.as_str()))
        .await;
    pane.type_line("print announced").await;
    pane.wait_for_text("announced").await;
    assert!(!pane.text().contains("prompt>"));
    pane
}

#[tokio::test]
async fn a_suspended_or_exited_pi_session_resumes_the_same_conversation() {
    let env = pi_env();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let repo = env.repo("app");
    let id = client.create(CreateSession::new(&repo, "Fix it")).await;
    client
        .until(&id, "held", |view| view.holder_pid.is_some())
        .await;

    kill(client.holder_pid(&id).unwrap(), "-KILL");
    client
        .until(&id, "Suspended", |view| view.phase == PhaseView::Suspended)
        .await;
    let mut pane = resumed_pane(&env, &mut client, &id).await;

    pane.type_line("exit 0").await;
    client
        .until(&id, "Exited", |view| {
            view.agent_state == Some(State::Exited)
        })
        .await;
    resumed_pane(&env, &mut client, &id).await;
}

#[tokio::test]
async fn an_edit_planted_in_the_profile_is_reset_at_the_next_launch() {
    let env = pi_env();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let repo = env.repo("app");
    let id = client.create(CreateSession::new(&repo, "Fix it")).await;
    client
        .until(&id, "held", |view| view.holder_pid.is_some())
        .await;
    let settings = env.path("state/orchestrator/agent-profiles/pi/settings.json");
    std::fs::write(&settings, r#"{"planted":true}"#).unwrap();
    let mut pane = env.pane(&id, PANE).await;
    pane.type_line("exit 0").await;
    client
        .until(&id, "Exited", |view| {
            view.agent_state == Some(State::Exited)
        })
        .await;

    resumed_pane(&env, &mut client, &id).await;

    let rebuilt = std::fs::read_to_string(&settings).unwrap();
    assert!(!rebuilt.contains("planted"), "{rebuilt}");
    assert!(rebuilt.contains("\"theme\": \"dark\""), "{rebuilt}");
}

fn pi_hook(payload: &str) -> String {
    format!("hook --agent pi {payload}")
}

async fn started_pi_session(
    env: &Env,
    client: &mut TestClient,
) -> (orch_core::SessionId, PaneView) {
    let repo = env.repo("app");
    let id = client.create(CreateSession::new(&repo, "Fix it")).await;
    client
        .until(&id, "Starting", |view| {
            view.agent_state == Some(State::Starting)
        })
        .await;
    let mut pane = env.pane(&id, PANE).await;
    pane.type_line(&pi_hook(&format!(
        r#"{{"event":"session_start","reason":"startup","sessionId":"{}"}}"#,
        id.as_str()
    )))
    .await;
    client
        .until(&id, "Idle", |view| view.agent_state == Some(State::Idle))
        .await;
    (id, pane)
}

async fn until_state(client: &mut TestClient, id: &orch_core::SessionId, state: State) {
    client
        .until(id, &format!("{state:?}"), |view| {
            view.agent_state == Some(state)
        })
        .await;
}

#[tokio::test]
async fn pi_hooks_drive_the_agent_state_through_a_prompt() {
    let env = pi_env();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = started_pi_session(&env, &mut client).await;

    for (payload, state) in [
        (r#"{"event":"input"}"#, State::Working),
        (
            r#"{"event":"tool_call","toolName":"question"}"#,
            State::NeedsInput,
        ),
        (
            r#"{"event":"tool_execution_end","toolName":"question","isError":false}"#,
            State::Working,
        ),
        (
            r#"{"event":"ui_prompt_start","kind":"confirm"}"#,
            State::NeedsInput,
        ),
        (
            r#"{"event":"ui_prompt_end","kind":"confirm","idle":false}"#,
            State::Working,
        ),
        (
            r#"{"event":"agent_settled","aborted":true,"stopReason":"aborted"}"#,
            State::Idle,
        ),
        (r#"{"event":"agent_start"}"#, State::Working),
        (
            r#"{"event":"agent_settled","aborted":false,"stopReason":"error","errorMessage":"429 rate limit"}"#,
            State::Errored,
        ),
    ] {
        pane.type_line(&pi_hook(payload)).await;
        until_state(&mut client, &id, state).await;
    }
}

#[tokio::test]
async fn a_new_conversation_in_pi_is_the_one_a_resume_continues() {
    let env = pi_env();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = started_pi_session(&env, &mut client).await;

    pane.type_line(&pi_hook(
        r#"{"event":"session_start","reason":"new","sessionId":"conv-2"}"#,
    ))
    .await;
    client
        .until(&id, "the new Conversation", |view| {
            view.conversation.as_deref() == Some("conv-2")
        })
        .await;
    pane.type_line("exit 0").await;
    until_state(&mut client, &id, State::Exited).await;

    let resumed = client
        .request(Request::Resume {
            session: id.clone(),
        })
        .await;
    assert_eq!(resumed, Ok(Reply::Done));
    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text("session-id> conv-2").await;
}

#[tokio::test]
async fn a_pi_session_name_is_the_title_until_it_is_cleared() {
    let env = pi_env();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = started_pi_session(&env, &mut client).await;

    pane.type_line(&pi_hook(
        r#"{"event":"session_info_changed","name":"fix login"}"#,
    ))
    .await;
    client
        .until(&id, "the title", |view| {
            view.title.as_deref() == Some("fix login")
        })
        .await;
    pane.type_line(&pi_hook(
        r#"{"event":"session_start","reason":"new","sessionId":"conv-2"}"#,
    ))
    .await;
    let renewed = client
        .until(&id, "the new Conversation", |view| {
            view.conversation.as_deref() == Some("conv-2")
        })
        .await;
    assert_eq!(renewed.title.as_deref(), Some("fix login"));

    pane.type_line(&pi_hook(r#"{"event":"session_info_changed","name":""}"#))
        .await;
    let cleared = client
        .until(&id, "the slug", |view| view.title.is_none())
        .await;
    assert_eq!(cleared.slug, "fix-it");
}

fn pi_tap(conversation: &str, (input, output, cache): (u64, u64, u64), cost: f64) -> String {
    format!(
        r#"tap --agent pi {{"sessionId":"{conversation}","model":{{"id":"claude-opus-5-5","name":"Opus 5.5"}},"context":{{"tokens":25000,"contextWindow":200000,"percent":12.5}},"lastResponse":{{"input":{input},"output":{output},"cacheRead":{cache},"cacheWrite":0}},"cost":{cost}}}"#
    )
}

async fn until_cost(client: &mut TestClient, id: &orch_core::SessionId, cost: f64) {
    client
        .until(id, &format!("cost {cost}"), |view| {
            view.cost_usd == Some(cost)
        })
        .await;
}

#[tokio::test]
async fn pi_taps_add_to_the_repos_usage_once_and_leave_the_rate_limits_alone() {
    let env = pi_env();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (pi, mut pi_pane) = started_pi_session(&env, &mut client).await;

    let orch = env!("CARGO_BIN_EXE_orch");
    env.write_config(&format!(
        "[defaults.agents.pi]\nbinary = {orch:?}\nargs = [\"fake-agent\", \"--\"]\n"
    ));
    let (_claude, mut claude_pane) = idle_session(&env, &mut client, "Claude work").await;
    claude_pane
        .type_line(
            r#"tap {"session_id":"claude-conv","context_window":{"total_input_tokens":100,"total_output_tokens":10},"cost":{"total_cost_usd":1.0},"rate_limits":{"five_hour":{"used_percentage":83},"seven_day":{"used_percentage":40}}}"#,
        )
        .await;
    client
        .until_received("rate limits", |client| !client.rate_limits.is_empty())
        .await;

    pi_pane
        .type_line(&pi_tap("conv-a", (100, 20, 300), 0.5))
        .await;
    let tapped = client
        .until(&pi, "pi's usage", |view| view.cost_usd == Some(0.5))
        .await;
    assert_eq!(tapped.context_used_percent, Some(12.5));
    pi_pane
        .type_line(&pi_tap("conv-a", (100, 20, 300), 0.5))
        .await;
    pi_pane
        .type_line(&pi_tap("conv-a", (200, 40, 600), 0.75))
        .await;
    until_cost(&mut client, &pi, 0.75).await;
    pi_pane
        .type_line(&pi_hook(
            r#"{"event":"session_start","reason":"new","sessionId":"conv-b"}"#,
        ))
        .await;
    pi_pane
        .type_line(&pi_tap("conv-b", (10, 1, 0), 0.125))
        .await;
    until_cost(&mut client, &pi, 0.125).await;

    let report = match client.request(Request::Usage).await {
        Ok(Reply::Usage(report)) => report,
        other => panic!("usage failed: {other:?}"),
    };
    let [app] = report.per_repo.as_slice() else {
        panic!("one Repo expected: {report:?}");
    };
    let UsageTotalsView {
        input_tokens,
        output_tokens,
        cost_usd,
    } = app.totals;
    assert_eq!((input_tokens, output_tokens), (100 + 800 + 10, 10 + 40 + 1));
    assert!((cost_usd - 1.875).abs() < 1e-9, "{cost_usd}");
    settled(&mut client).await;
    assert_eq!(
        client.rate_limits,
        [RateLimits {
            five_hour: Some(83.0),
            seven_day: Some(40.0)
        }]
    );
}

fn real_pi() -> Option<std::path::PathBuf> {
    if std::env::var("ORCH_REAL_PI").as_deref() != Ok("1") {
        return None;
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("pi"))
        .find(|pi| pi.is_file())
}

#[tokio::test]
async fn real_pi_loads_orchs_extension_reports_its_start_and_resumes() {
    let Some(pi) = real_pi() else {
        return;
    };
    let env = Env::new();
    env.write_config(&format!("{PI}[defaults.agents.pi]\nbinary = {pi:?}\n"));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let repo = env.repo("app");
    let id = client.create(CreateSession::new(&repo, "")).await;

    let started = client
        .until(&id, "pi's session_start", |view| {
            view.agent_state == Some(State::Idle)
        })
        .await;
    assert_eq!(started.conversation.as_deref(), Some(id.as_str()));
    let profile = env.path("state/orchestrator/agent-profiles/pi");
    assert!(profile.join("extensions/orch/extension/index.ts").is_file());

    kill(client.holder_pid(&id).unwrap(), "-KILL");
    client
        .until(&id, "Suspended", |view| view.phase == PhaseView::Suspended)
        .await;
    let resumed = client
        .request(Request::Resume {
            session: id.clone(),
        })
        .await;
    assert_eq!(resumed, Ok(Reply::Done));
    let back = client
        .until(&id, "pi's session_start after the resume", |view| {
            view.phase == PhaseView::Active && view.agent_state == Some(State::Idle)
        })
        .await;
    assert_eq!(back.conversation.as_deref(), Some(id.as_str()));
}

fn child_said(text: &str) -> String {
    format!(
        r#"{{"role":"assistant","content":[{{"type":"thinking","thinking":"hm"}},{{"type":"text","text":"{text}"}}]}}"#
    )
}

fn write_transcript(path: &std::path::Path, lines: &[String]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let staged = path.with_extension("jsonl.new");
    std::fs::write(
        &staged,
        lines
            .iter()
            .map(|line| format!("{line}\n"))
            .collect::<String>(),
    )
    .unwrap();
    std::fs::rename(staged, path).unwrap();
}

#[tokio::test]
async fn a_pi_subagent_call_is_a_subagent_row_with_a_live_transcript_that_outlives_it() {
    let env = pi_env();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = started_pi_session(&env, &mut client).await;
    let conversation = id.as_str();

    pane.type_line(&pi_hook(&format!(
        r#"{{"event":"tool_call","toolName":"subagent","toolCallId":"call_7","sessionId":"{conversation}","agent":"scout","taskName":"find the redirect"}}"#
    )))
    .await;
    let view = client
        .until(&id, "a Subagent", |view| !view.subagents.is_empty())
        .await;
    let row = &view.subagents[0];
    assert_eq!(
        (
            row.id.as_str(),
            row.agent_type.as_str(),
            row.description.as_str(),
            row.done
        ),
        ("call_7", "scout", "find the redirect", false)
    );
    assert_eq!(view.agent_state, Some(State::Working));

    let transcript = env.path(&format!(
        "state/orchestrator/agent-profiles/pi/subagents/{conversation}/call_7.jsonl"
    ));
    let task = r#"{"role":"user","content":[{"type":"text","text":"Task: find the redirect"}]}"#;
    write_transcript(&transcript, &[task.into(), child_said("Overloaded.")]);
    let subscribe = Request::SubscribeSubagent {
        session: id.clone(),
        subagent: "call_7".into(),
    };
    client.request(subscribe.clone()).await.unwrap();
    client
        .until_received("the backlog", |client| !client.transcripts.is_empty())
        .await;
    assert_eq!(
        client.transcripts[0].entries,
        [
            TranscriptEntry::Prompt {
                text: "Task: find the redirect".into()
            },
            TranscriptEntry::Text {
                text: "Overloaded.".into()
            },
        ]
    );

    write_transcript(&transcript, &[task.into()]);
    client
        .until_received("the retry", |client| client.transcripts.len() >= 2)
        .await;
    assert!(client.transcripts[1].replace);
    assert_eq!(client.transcripts[1].entries.len(), 1);

    pane.type_line(&pi_hook(
        r#"{"event":"tool_execution_end","toolName":"subagent","toolCallId":"call_7"}"#,
    ))
    .await;
    client
        .until(&id, "the Subagent done", |view| {
            view.subagents.iter().all(|subagent| subagent.done)
        })
        .await;
    client.request(Request::UnsubscribeSubagent).await.unwrap();
    client.drain().await;
    client.transcripts.clear();
    client.request(subscribe).await.unwrap();
    client
        .until_received("the finished transcript", |client| {
            !client.transcripts.is_empty()
        })
        .await;
    assert!(client.transcripts[0].replace);
    assert_eq!(client.transcripts[0].entries.len(), 1);
}

fn live_conversation(env: &Env, worktree: &std::path::Path, id: &orch_core::SessionId) -> PathBuf {
    let dir = orch_agent::pi_session_dir_name(worktree);
    let file = env
        .path("state/orchestrator/agent-profiles/pi/sessions")
        .join(dir)
        .join(format!("2026-10-30T09-00-00-000Z_{}.jsonl", id.as_str()));
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "the live conversation\n").unwrap();
    file
}

fn flag_value(text: &str, flag: &str) -> PathBuf {
    let at = text
        .find(flag)
        .unwrap_or_else(|| panic!("{flag} in {text}"));
    text[at + flag.len()..]
        .split_whitespace()
        .next()
        .unwrap()
        .into()
}

async fn draft(client: &mut TestClient, id: &orch_core::SessionId) -> Result<Reply, String> {
    client
        .request(Request::Draft {
            session: id.clone(),
            mode: orch_protocol::LandingMode::Squash,
        })
        .await
        .map_err(|err| match err {
            orch_protocol::RequestError::Refused { message } => message,
            other => format!("{other:?}"),
        })
}

#[tokio::test]
async fn a_pi_draft_forks_the_conversation_and_reads_the_base_diff_from_stdin() {
    let env = pi_env();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = started_pi_session(&env, &mut client).await;
    let worktree = client.sessions[&id].worktree.clone();
    let live = live_conversation(&env, &worktree, &id);
    commit(&worktree, "committed.txt", "on the branch\n");
    std::fs::write(worktree.join("untracked.txt"), "not yet added\n").unwrap();

    let Ok(Reply::Drafted { title, body }) = draft(&mut client, &id).await else {
        panic!("pi drafts");
    };

    assert_eq!(title, "Drafted from the live conversation");
    assert!(
        body.contains("-p -ne -ns -np --tools read,grep,find,ls --fork "),
        "{body}"
    );
    let profile = env.path("state/orchestrator/agent-profiles/pi");
    assert!(
        body.contains(&format!("PI_CODING_AGENT_DIR={}", profile.display())),
        "{body}"
    );
    assert!(body.contains("commit message"), "{body}");
    assert!(body.contains("?? untracked.txt"), "{body}");
    assert!(body.contains("+on the branch"), "{body}");
    assert!(body.contains("+not yet added"), "{body}");
    let fork = flag_value(&body, "--fork ");
    assert_ne!(fork, live);
    assert!(!fork.exists());
    let sessions = flag_value(&body, "--session-dir ");
    assert!(sessions.ends_with("sessions"), "{body}");
    assert!(!sessions.parent().unwrap().exists());
    assert_eq!(
        std::fs::read_to_string(&live).unwrap(),
        "the live conversation\n"
    );
}

#[tokio::test]
async fn a_failing_pi_draft_still_leaves_no_fork_behind() {
    let env = pi_env();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = started_pi_session(&env, &mut client).await;
    let worktree = client.sessions[&id].worktree.clone();
    live_conversation(&env, &worktree, &id);
    std::fs::write(worktree.join("note.txt"), "fail the draft\n").unwrap();

    let failed = draft(&mut client, &id).await.unwrap_err();

    assert!(failed.contains("fake-agent cannot draft"), "{failed}");
    let sessions = flag_value(&failed, "--session-dir ");
    assert!(sessions.ends_with("sessions"), "{failed}");
    assert!(!sessions.parent().unwrap().exists());
}

#[tokio::test]
async fn without_its_conversation_file_a_pi_session_gets_the_default_draft() {
    let env = pi_env();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = started_pi_session(&env, &mut client).await;

    let drafted = draft(&mut client, &id).await;

    assert_eq!(
        drafted,
        Ok(Reply::Drafted {
            title: "Fix it".into(),
            body: String::new(),
        })
    );
}

fn big_change(worktree: &std::path::Path) {
    std::fs::write(
        worktree.join("big.txt"),
        "a changed line\n".repeat(64 * 1024),
    )
    .unwrap();
}

#[tokio::test]
async fn a_pi_draft_reads_at_most_a_capped_base_diff() {
    let env = pi_env();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = started_pi_session(&env, &mut client).await;
    let worktree = client.sessions[&id].worktree.clone();
    live_conversation(&env, &worktree, &id);
    big_change(&worktree);

    let Ok(Reply::Drafted { body, .. }) = draft(&mut client, &id).await else {
        panic!("pi drafts");
    };

    assert!(
        body.contains("[the diff is cut off here]"),
        "{}",
        body.len()
    );
    assert!(body.len() < 300 * 1024, "{}", body.len());
}

#[tokio::test]
async fn a_pi_draft_that_never_reads_its_input_times_out_and_leaves_no_fork_behind() {
    let env = pi_env();
    let _daemon = env
        .start_daemon_args(Duration::from_secs(600), &["--draft-timeout-ms", "1500"])
        .await;
    let mut client = env.client().await;
    let (id, _pane) = started_pi_session(&env, &mut client).await;
    let worktree = client.sessions[&id].worktree.clone();
    live_conversation(&env, &worktree, &id);
    big_change(&worktree);
    std::fs::write(worktree.join("hang-draft"), "").unwrap();

    let failed = tokio::time::timeout(Duration::from_secs(20), draft(&mut client, &id))
        .await
        .expect("the Draft times out")
        .unwrap_err();

    assert!(failed.contains("took longer"), "{failed}");
    let args = std::fs::read_to_string(worktree.join("hang-draft")).unwrap();
    let sessions = flag_value(&args, "--session-dir ");
    assert!(sessions.ends_with("sessions"), "{args}");
    assert!(!sessions.parent().unwrap().exists());
}
