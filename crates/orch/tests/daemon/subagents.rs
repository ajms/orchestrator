use std::io::Write;
use std::path::{Path, PathBuf};

use orch_core::TranscriptEntry;
use orch_holder::{read_frame_async, write_frame_async};
use orch_protocol::{DisplayVars, FromDaemon, Request, RequestError, SubagentTranscript, ToDaemon};
use tokio::io::AsyncWriteExt;

use crate::common::*;

const SUBAGENT: &str = "a7f3";

fn subagent_hook(event: &str, extra: &str) -> String {
    hook_for(SUBAGENT, event, extra)
}

fn hook_for(subagent: &str, event: &str, extra: &str) -> String {
    let extra = if extra.is_empty() {
        String::new()
    } else {
        format!(",{extra}")
    };
    hook(
        event,
        &format!(r#""agent_id":"{subagent}","agent_type":"Explore"{extra}"#),
    )
}

fn reading(transcript: &Path) -> String {
    format!(r#""transcript_path":"{}""#, transcript.display())
}

fn append(path: &Path, line: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = std::fs::File::options()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    writeln!(file, "{line}").unwrap();
}

fn said(text: &str) -> String {
    format!(
        r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"{text}"}}]}}}}"#
    )
}

fn text(text: &str) -> TranscriptEntry {
    TranscriptEntry::Text { text: text.into() }
}

async fn subscribe(client: &mut TestClient, session: &orch_core::SessionId) {
    subscribe_to(client, session, SUBAGENT).await;
}

async fn subscribe_to(client: &mut TestClient, session: &orch_core::SessionId, subagent: &str) {
    let subscribe = Request::SubscribeSubagent {
        session: session.clone(),
        subagent: subagent.into(),
    };
    client.request(subscribe).await.unwrap();
}

async fn until_transcript(client: &mut TestClient, count: usize) -> Vec<SubagentTranscript> {
    client
        .until_received("the Subagent transcript", |client| {
            client.transcripts.len() >= count
        })
        .await;
    client.transcripts.clone()
}

async fn started_subagent(
    env: &Env,
    client: &mut TestClient,
) -> (orch_core::SessionId, PaneView, PathBuf) {
    let (id, mut pane) = idle_session(env, client, "Explore").await;
    let conversation = env.path("projects/conv-1.jsonl");
    pane.hook(&subagent_hook("SubagentStart", &reading(&conversation)))
        .await;
    client
        .until(&id, "a Subagent", |view| !view.subagents.is_empty())
        .await;
    let transcript = env.path(&format!("projects/conv-1/subagents/agent-{SUBAGENT}.jsonl"));
    (id, pane, transcript)
}

#[tokio::test]
async fn subscribing_to_a_subagent_streams_its_backlog_then_appended_entries() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane, transcript) = started_subagent(&env, &mut client).await;
    append(
        &transcript,
        r#"{"type":"user","message":{"role":"user","content":"Find the redirect"}}"#,
    );
    append(&transcript, &said("Searching."));

    subscribe(&mut client, &id).await;
    let backlog = until_transcript(&mut client, 1).await;
    assert_eq!(
        backlog[0],
        SubagentTranscript {
            session: id.clone(),
            subagent: SUBAGENT.into(),
            entries: vec![
                TranscriptEntry::Prompt {
                    text: "Find the redirect".into()
                },
                text("Searching."),
            ],
            replace: true,
        }
    );

    append(&transcript, &said("Found it."));
    let streamed = until_transcript(&mut client, 2).await;
    assert_eq!(streamed[1].entries, [text("Found it.")]);
    assert!(!streamed[1].replace);
}

#[tokio::test]
async fn a_done_subagent_stays_available_from_the_transcript_subagent_stop_names() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane, _) = started_subagent(&env, &mut client).await;
    let named = env.path("elsewhere/agent.jsonl");
    append(&named, &said("Done."));
    pane.hook(&subagent_hook(
        "SubagentStop",
        &format!(r#""agent_transcript_path":"{}""#, named.display()),
    ))
    .await;
    client
        .until(&id, "the Subagent done", |view| {
            view.subagents.iter().all(|subagent| subagent.done)
        })
        .await;

    subscribe(&mut client, &id).await;
    let backlog = until_transcript(&mut client, 1).await;
    assert_eq!(backlog[0].entries, [text("Done.")]);
    assert!(backlog[0].replace);
}

#[tokio::test]
async fn an_unknown_subagent_cannot_be_subscribed_to() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = idle_session(&env, &mut client, "Explore").await;
    let subscribe = Request::SubscribeSubagent {
        session: id,
        subagent: "nobody".into(),
    };
    assert!(matches!(
        client.request(subscribe).await,
        Err(RequestError::Refused { .. })
    ));
}

#[tokio::test]
async fn unsubscribing_stops_the_stream() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane, transcript) = started_subagent(&env, &mut client).await;
    append(&transcript, &said("Searching."));
    subscribe(&mut client, &id).await;
    until_transcript(&mut client, 1).await;

    client.request(Request::UnsubscribeSubagent).await.unwrap();
    append(&transcript, &said("Found it."));
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    client.drain().await;
    assert_eq!(client.transcripts.len(), 1);
}

#[tokio::test]
async fn subscribing_to_another_subagent_replaces_the_stream() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane, first) = started_subagent(&env, &mut client).await;
    append(&first, &said("First."));
    subscribe(&mut client, &id).await;
    until_transcript(&mut client, 1).await;

    let conversation = env.path("projects/conv-1.jsonl");
    pane.hook(&hook_for("b9e2", "SubagentStart", &reading(&conversation)))
        .await;
    client
        .until(&id, "a second Subagent", |view| view.subagents.len() == 2)
        .await;
    append(
        &env.path("projects/conv-1/subagents/agent-b9e2.jsonl"),
        &said("Second."),
    );
    subscribe_to(&mut client, &id, "b9e2").await;
    let streamed = until_transcript(&mut client, 2).await;
    assert_eq!(streamed[1].subagent, "b9e2");
    assert_eq!(streamed[1].entries, [text("Second.")]);
    assert!(streamed[1].replace);

    append(&first, &said("Still first."));
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    client.drain().await;
    assert_eq!(client.transcripts.len(), 2, "{:?}", client.transcripts);
}

#[tokio::test]
async fn back_to_back_subscriptions_follow_the_last_one() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane, first) = started_subagent(&env, &mut client).await;
    append(&first, &said("First."));
    let conversation = env.path("projects/conv-1.jsonl");
    pane.hook(&hook_for("b9e2", "SubagentStart", &reading(&conversation)))
        .await;
    client
        .until(&id, "a second Subagent", |view| view.subagents.len() == 2)
        .await;
    append(
        &env.path("projects/conv-1/subagents/agent-b9e2.jsonl"),
        &said("Second."),
    );

    let (mut reader, mut writer) = env.tui(DisplayVars::default()).await;
    let mut both = Vec::new();
    for (n, subagent) in [SUBAGENT, "b9e2"].repeat(5).into_iter().enumerate() {
        let request = Request::SubscribeSubagent {
            session: id.clone(),
            subagent: subagent.into(),
        };
        write_frame_async(
            &mut both,
            &ToDaemon::Request {
                id: n as u64,
                request,
            },
        )
        .await
        .unwrap();
    }
    writer.write_all(&both).await.unwrap();
    let mut last = None;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(800);
    while let Ok(frame) =
        tokio::time::timeout_at(deadline, read_frame_async::<FromDaemon>(&mut reader)).await
    {
        if let Some(FromDaemon::SubagentTranscript(streamed)) = frame.unwrap() {
            last = Some(streamed.subagent);
        }
    }
    assert_eq!(last.as_deref(), Some("b9e2"));
}
