use std::time::{Duration, Instant};

use orch_agent::{AgentAdapter, Antigravity, Draft};
use orch_holder::{FromHolder, ToHolder};
use orch_protocol::{
    AgentStateView as State, GuardKindView, LandingMode, PhaseView, Reply, Request, TranscriptEntry,
};

use crate::agy::*;
use crate::common::*;

const READY: &str = "Reply with the single word ready and nothing else.";

fn window(argv: &[String], pair: [&str; 2]) -> bool {
    argv.windows(2).any(|window| window == pair)
}

#[tokio::test]
#[ignore = "runs the real agy; set ORCH_REAL_AGY=1"]
async fn each_fresh_worktree_needs_input_on_agys_trust_screen() {
    let Some(agy) = RealAgy::new().await else {
        return;
    };
    let _daemon = agy.start_daemon().await;
    let mut client = agy.env.client().await;
    let repo = agy.env.repo("app");

    for _ in 0..2 {
        let (id, mut pane) = agy.session(&mut client, &repo, "edits", READY).await;
        answer_trust(&mut client, &id, &mut pane).await;
        until_state(&mut client, &id, State::Idle, TURN).await;
        agy.record(&client, &id);
    }
}

#[tokio::test]
#[ignore = "runs the real agy; set ORCH_REAL_AGY=1"]
async fn the_initial_prompt_reaches_orch_through_the_hookup_until_a_fully_idle_stop() {
    let Some(agy) = RealAgy::new().await else {
        return;
    };
    let _daemon = agy.start_daemon().await;
    let mut client = agy.env.client().await;
    let repo = agy.env.repo("app");
    let outside = agy.env.path("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let target = outside.join("guard-check");
    let prompt = format!(
        "Run exactly this shell command with your run_command tool: touch {}. If it is denied, do not retry and do nothing else.",
        target.display()
    );

    let (id, mut pane) = agy.session(&mut client, &repo, "ask", &prompt).await;
    agy.agy_argv("-i and the prompt", |argv| window(argv, ["-i", &prompt]))
        .await;
    answer_trust(&mut client, &id, &mut pane).await;
    let denied = deny_guards_until_idle(&mut client, &id).await;

    assert_eq!(
        denied.first().map(|guard| guard.kind),
        Some(GuardKindView::WriteOutsideWorktree),
        "{denied:?}"
    );
    assert!(!target.exists());
    agy.record(&client, &id);
}

#[tokio::test]
#[ignore = "runs the real agy; set ORCH_REAL_AGY=1"]
async fn a_permission_prompt_needs_input_until_the_user_answers_it() {
    let Some(agy) = RealAgy::new().await else {
        return;
    };
    let _daemon = agy.start_daemon().await;
    let mut client = agy.env.client().await;
    let repo = agy.env.repo("app");
    let prompt = "Run exactly this shell command with your run_command tool in the workspace root and nothing else: touch permission-granted";

    let (id, mut pane) = agy.session(&mut client, &repo, "edits", prompt).await;
    answer_trust(&mut client, &id, &mut pane).await;
    until_state(&mut client, &id, State::NeedsInput, TURN).await;
    let worktree = client.sessions[&id].worktree.clone();
    assert!(!worktree.join("permission-granted").exists());

    pane.pane.input(PERMISSION_ANSWER.to_vec()).await.unwrap();
    until_state(&mut client, &id, State::Idle, TURN).await;

    assert!(worktree.join("permission-granted").exists());
    agy.record(&client, &id);
}

#[tokio::test]
#[ignore = "runs the real agy; set ORCH_REAL_AGY=1"]
async fn resume_reopens_the_conversation_in_the_mode_cycled_with_shift_tab() {
    let Some(agy) = RealAgy::new().await else {
        return;
    };
    let _daemon = agy.start_daemon().await;
    let mut client = agy.env.client().await;
    let repo = agy.env.repo("app");
    let (id, mut pane) = agy.session(&mut client, &repo, "edits", READY).await;
    answer_trust(&mut client, &id, &mut pane).await;
    until_state(&mut client, &id, State::Idle, TURN).await;
    let conversation = client.sessions[&id]
        .conversation
        .clone()
        .expect("agy's statusline names the Conversation");

    pane.pane.input(SHIFT_TAB.to_vec()).await.unwrap();
    client
        .until_within(&id, "plan mode", STARTUP, |view| {
            view.mode.as_deref() == Some("plan")
        })
        .await;
    agy.record(&client, &id);
    kill(client.holder_pid(&id).unwrap(), "-KILL");
    client
        .until_within(&id, "Suspended", STARTUP, |view| {
            view.phase == PhaseView::Suspended
        })
        .await;
    let resumed = client
        .request(Request::Resume {
            session: id.clone(),
        })
        .await;
    assert_eq!(resumed, Ok(Reply::Done));

    let argv = agy
        .agy_argv("--conversation", |argv| {
            argv.iter().any(|arg| arg == "--conversation")
        })
        .await;
    assert!(window(&argv, ["--conversation", &conversation]), "{argv:?}");
    assert!(window(&argv, ["--mode", "plan"]), "{argv:?}");
    assert!(!argv.iter().any(|arg| arg == "-i"), "{argv:?}");
    let back = client
        .until_within(&id, "Idle after the resume", STARTUP, |view| {
            view.phase == PhaseView::Active && view.agent == Some(State::Idle)
        })
        .await;
    assert_eq!(back.conversation.as_deref(), Some(conversation.as_str()));
    assert_eq!(back.mode.as_deref(), Some("plan"));
}

#[tokio::test]
#[ignore = "runs the real agy; set ORCH_REAL_AGY=1"]
async fn a_subagent_becomes_a_subagent_row_with_its_transcript() {
    let Some(agy) = RealAgy::new().await else {
        return;
    };
    let _daemon = agy.start_daemon().await;
    let mut client = agy.env.client().await;
    let repo = agy.env.repo("app");
    let prompt = "Use your invoke_subagent tool to start exactly one subagent. Its task: reply with the single word pong and nothing else. Wait for its reply, then tell me what it said.";

    let (id, mut pane) = agy.session(&mut client, &repo, "edits", prompt).await;
    answer_trust(&mut client, &id, &mut pane).await;
    let finished = client
        .until_within(&id, "a finished Subagent row", TURN, |view| {
            view.subagents.iter().any(|row| row.done)
        })
        .await;
    assert_eq!(finished.subagents.len(), 1, "{:?}", finished.subagents);
    let subscribe = Request::SubscribeSubagent {
        session: id.clone(),
        subagent: finished.subagents[0].id.clone(),
    };
    client.request(subscribe).await.unwrap();
    client
        .until_received("the Subagent transcript", |client| {
            !client.transcripts.is_empty()
        })
        .await;

    let asked = client.transcripts[0]
        .entries
        .iter()
        .any(|entry| matches!(entry, TranscriptEntry::Prompt { text } if text.contains("pong")));
    assert!(asked, "{:?}", client.transcripts[0].entries);
    until_state(&mut client, &id, State::Idle, TURN).await;
    agy.record(&client, &id);
}

#[tokio::test]
#[ignore = "runs the real agy; set ORCH_REAL_AGY=1"]
async fn agys_print_mode_reads_its_prompt_from_stdin() {
    let Some(agy) = RealAgy::new().await else {
        return;
    };
    let repo = agy.env.repo("app");
    let adapter = Antigravity {
        program: agy.agy.display().to_string(),
    };
    let Some(Draft { argv, .. }) = adapter.draft(None) else {
        panic!("Antigravity drafts");
    };
    let mut command = agy.command(&argv.program);
    command.args(&argv.args).current_dir(&repo);

    let output = run_with_stdin(
        command,
        "Reply with the single word pineapple and nothing else.",
        TURN,
    );

    let stdout = String::from_utf8_lossy(&output.stdout).to_lowercase();
    assert!(
        output.status.success() && stdout.contains("pineapple"),
        "agy {:?} did not answer the prompt on stdin: {output:?}",
        argv.args
    );
}

#[tokio::test]
#[ignore = "runs the real agy; set ORCH_REAL_AGY=1"]
async fn a_draft_comes_from_a_fresh_headless_agy_over_the_base_diff() {
    let Some(agy) = RealAgy::new().await else {
        return;
    };
    let _daemon = agy.start_daemon().await;
    let mut client = agy.env.client().await;
    let repo = agy.env.repo("app");
    let (id, mut pane) = agy.session(&mut client, &repo, "edits", READY).await;
    answer_trust(&mut client, &id, &mut pane).await;
    until_state(&mut client, &id, State::Idle, TURN).await;
    let worktree = client.sessions[&id].worktree.clone();
    std::fs::write(
        worktree.join("greeting.txt"),
        "Hello from the greeting module\n",
    )
    .unwrap();

    let draft = Request::Draft {
        session: id.clone(),
        mode: LandingMode::Squash,
    };
    let drafted = client.request_within(draft, TURN).await;

    let Ok(Reply::Drafted { title, body }) = drafted else {
        panic!("no draft: {drafted:?}");
    };
    assert!(!title.is_empty() && title != READY, "{title}");
    assert!(
        format!("{title}\n{body}")
            .to_lowercase()
            .contains("greeting"),
        "{title}\n{body}"
    );
}

#[tokio::test]
#[ignore = "runs the real agy; set ORCH_REAL_AGY=1"]
async fn agys_osc_11_query_does_not_stall_its_start_in_the_holder() {
    let Some(agy) = RealAgy::new().await else {
        return;
    };
    let repo = agy.env.repo("app");
    let started = Instant::now();
    let mut held = agy.hold("osc11", &repo).await;
    held.client.send(&ToHolder::Subscribe).await.unwrap();
    let mut chunks = Vec::new();
    let mut screen = vt100::Parser::new(24, 80, 0);

    while !is_trust_screen(&screen.screen().contents().to_lowercase()) {
        let remaining = STARTUP.saturating_sub(started.elapsed());
        let message = tokio::time::timeout(remaining, held.client.recv()).await;
        let Ok(Ok(Some(message))) = message else {
            panic!(
                "agy never showed its trust screen in the Holder:\n{}",
                screen.screen().contents()
            );
        };
        match message {
            FromHolder::Screen(snapshot) => screen = snapshot.restore(0),
            FromHolder::Output { bytes } => {
                screen.process(&bytes);
                chunks.push((started.elapsed(), bytes));
            }
            _ => {}
        }
    }
    let trusted_at = started.elapsed();

    let stall = osc_11_stall(&chunks, trusted_at);
    eprintln!(
        "OSC 11 finding: query (asked at, silence after) = {stall:?}; trust screen after {trusted_at:?}"
    );
    if let Some((_, silence)) = stall {
        assert!(silence < Duration::from_secs(1), "{silence:?}");
    }
}
