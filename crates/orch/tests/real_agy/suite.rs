use std::path::Path;
use std::time::Duration;

use orch_agent::{AgentAdapter, Antigravity, Draft, DraftOutcome};
use orch_core::ConversationId;
use orch_protocol::{
    AgentStateView as State, GuardKindView, LandingMode, PhaseView, Reply, Request, TranscriptEntry,
};

use crate::agy::*;
use crate::common::*;
use crate::fixtures::Recording;
use crate::osc;

const READY: &str = "Reply with the single word ready and nothing else.";
const TRUST_DWELL: Duration = Duration::from_secs(3);

fn window(argv: &[String], pair: [&str; 2]) -> bool {
    argv.windows(2).any(|window| window == pair)
}

#[tokio::test]
#[ignore = "runs the real agy; set ORCH_REAL_AGY=1"]
async fn each_fresh_worktree_shows_agys_trust_screen_while_the_session_is_starting() {
    let Some(agy) = RealAgy::new().await else {
        return;
    };
    let _daemon = agy.start_daemon().await;
    let mut client = agy.env.client().await;
    let repo = agy.env.repo("app");

    for first in [true, false] {
        let (id, mut pane) = agy.session(&mut client, &repo, "edits", READY).await;
        wait_for_trust_screen(&mut pane).await;
        tokio::time::sleep(TRUST_DWELL).await;
        settled(&mut client).await;
        let trusting = &client.sessions[&id];
        assert_eq!(trusting.agent, Some(State::Starting), "{}", pane.text());
        let needed_input = client
            .history
            .iter()
            .any(|view| view.id == id && view.agent == Some(State::NeedsInput));
        assert!(!needed_input, "Needs input on the trust screen");

        answer_trust(&mut pane).await;
        until_state(&mut client, &id, State::Idle, TURN).await;
        if first {
            agy.record(&client, &id, Recording::Trust);
        }
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
    let outside = tempfile::Builder::new()
        .prefix("real-agy-outside")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let target = outside.path().join("guard-check");
    let prompt = format!(
        "Run exactly this shell command with your run_command tool: touch {}. If it is denied, do not retry and do nothing else.",
        target.display()
    );

    let (id, mut pane) = agy.session(&mut client, &repo, "ask", &prompt).await;
    agy.agy_argv("-i and the prompt", |argv| window(argv, ["-i", &prompt]))
        .await;
    answer_trust(&mut pane).await;
    let denied = deny_guards_until_idle(&mut client, &id).await;

    assert_eq!(
        denied.first().map(|guard| guard.kind),
        Some(GuardKindView::WriteOutsideWorktree),
        "{denied:?}"
    );
    assert!(!target.exists());
    let root = agy.conversations(&client, &id).root;
    assert!(
        agy.saw_fully_idle_stop(&id, &root).await,
        "no Stop with fullyIdle: true from {root}; Stops seen: {:?}",
        agy.stops(&id)
    );
    agy.record(&client, &id, Recording::Hookup);
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
    answer_trust(&mut pane).await;
    until_state(&mut client, &id, State::NeedsInput, TURN).await;
    let worktree = client.sessions[&id].worktree.clone();
    assert!(!worktree.join("permission-granted").exists());

    pane.pane.input(PERMISSION_ANSWER.to_vec()).await.unwrap();
    until_state(&mut client, &id, State::Idle, TURN).await;

    assert!(worktree.join("permission-granted").exists());
    agy.record(&client, &id, Recording::Permission);
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
    answer_trust(&mut pane).await;
    until_state(&mut client, &id, State::Idle, TURN).await;
    let conversation = agy.conversations(&client, &id).root;

    pane.pane.input(SHIFT_TAB.to_vec()).await.unwrap();
    client
        .until_within(&id, "plan mode", STARTUP, |view| {
            view.mode.as_deref() == Some("plan")
        })
        .await;
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
    answer_trust(&mut pane).await;
    let finished = client
        .until_within(&id, "a finished Subagent row", TURN, |view| {
            view.subagents.iter().any(|row| row.done)
        })
        .await;
    assert_eq!(finished.subagents.len(), 1, "{:?}", finished.subagents);
    let row = finished.subagents[0].id.clone();
    let subscribe = Request::SubscribeSubagent {
        session: id.clone(),
        subagent: row.clone(),
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

    let conversations = agy.conversations(&client, &id);
    assert_eq!(conversations.subagents, [row]);
    let idle_while_running = client.history.iter().any(|view| {
        view.id == id
            && view.agent == Some(State::Idle)
            && view.subagents.iter().any(|row| !row.done)
    });
    assert!(!idle_while_running, "Idle while a Subagent row ran");
    assert!(
        agy.saw_fully_idle_stop(&id, &conversations.root).await,
        "no Stop with fullyIdle: true from {}; Stops seen: {:?}",
        conversations.root,
        agy.stops(&id)
    );
    agy.record(&client, &id, Recording::Subagent);
}

#[tokio::test]
#[ignore = "runs the real agy; set ORCH_REAL_AGY=1"]
async fn agys_stream_json_print_mode_answers_the_user_event_on_stdin() {
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
        &adapter.encode_draft("Reply with the single word pineapple and nothing else."),
        TURN,
    );

    let response = adapter.decode_draft(&String::from_utf8_lossy(&output.stdout));
    let answered = matches!(&response, DraftOutcome::Drafted(text) if text.to_lowercase().contains("pineapple"));
    assert!(
        output.status.success() && answered,
        "agy {:?} did not answer the user event on stdin with a result: {response:?} from {output:?}",
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
    answer_trust(&mut pane).await;
    until_state(&mut client, &id, State::Idle, TURN).await;
    let conversation = agy.conversations(&client, &id).root;
    let transcript = agy.transcript(&conversation);
    assert!(!transcript.is_empty(), "no transcript for {conversation}");
    let worktree = client.sessions[&id].worktree.clone();
    std::fs::write(
        worktree.join("greeting.txt"),
        "Hello from the greeting module\n",
    )
    .unwrap();
    let adapter = Antigravity {
        program: agy.agy.display().to_string(),
    };
    let current = ConversationId(conversation.clone());
    let draft_argv = adapter.draft(Some(&current)).unwrap().argv.args;
    assert!(
        !draft_argv.iter().any(|arg| arg == "--conversation"),
        "{draft_argv:?}"
    );

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
    settled(&mut client).await;
    assert_eq!(
        client.sessions[&id].conversation.as_deref(),
        Some(conversation.as_str())
    );
    assert_eq!(agy.transcript(&conversation), transcript);
}

#[tokio::test]
#[ignore = "runs the real agy; set ORCH_REAL_AGY=1"]
async fn agys_osc_11_query_does_not_stall_its_start_in_the_holder() {
    let Some(agy) = RealAgy::new().await else {
        return;
    };
    let repo = agy.env.repo("app");

    let unanswered = osc::probe(&agy, "osc11", &repo, false).await;
    let answered = osc::probe(&agy, "osc11-answered", &repo, true).await;

    let finding = osc::finding(&unanswered, &answered);
    let file = Path::new(env!("CARGO_TARGET_TMPDIR")).join("real-agy-osc11.txt");
    std::fs::write(&file, &finding).unwrap();
    report(&format!("{finding}(saved to {})\n", file.display()));
    if let Some((_, silence)) = unanswered.asked {
        assert!(silence < Duration::from_secs(1), "{finding}");
    }
}
