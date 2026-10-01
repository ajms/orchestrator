use std::io::Write;
use std::path::Path;

use crate::common::*;

fn titled(title: &str) -> String {
    format!(r#""session_title":"{title}""#)
}

fn reading(transcript: &Path) -> String {
    format!(r#""transcript_path":"{}""#, transcript.display())
}

fn rename(transcript: &Path, title: &str) {
    let mut file = std::fs::File::options()
        .create(true)
        .append(true)
        .open(transcript)
        .unwrap();
    writeln!(
        file,
        r#"{{"type":"custom-title","customTitle":"{title}","sessionId":"conv-1"}}"#
    )
    .unwrap();
}

#[tokio::test]
async fn a_title_given_through_the_agent_outlives_clear_and_a_daemon_restart() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut client, "Fix login").await;

    pane.hook(&hook("UserPromptSubmit", &titled("login redirect")))
        .await;
    let titled_view = client
        .until(&id, "the title", |view| {
            view.title.as_deref() == Some("login redirect")
        })
        .await;
    assert_eq!(titled_view.slug, "fix-login");

    pane.hook(r#"{"hook_event_name":"SessionStart","session_id":"conv-2","source":"clear"}"#)
        .await;
    let cleared = client
        .until(&id, "the new Conversation", |view| {
            view.conversation.as_deref() == Some("conv-2")
        })
        .await;
    assert_eq!(cleared.title.as_deref(), Some("login redirect"));
    settled(&mut client).await;

    daemon.kill();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let listed = client.session_list().await;
    assert_eq!(listed[0].title.as_deref(), Some("login redirect"));
}

#[tokio::test]
async fn a_rename_in_the_transcript_replaces_the_title_and_an_empty_one_brings_the_slug_back() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut client, "Fix login").await;
    let transcript = env.path("transcript.jsonl");

    rename(&transcript, "login loop");
    pane.hook(&hook("Stop", &reading(&transcript))).await;
    client
        .until(&id, "the first title", |view| {
            view.title.as_deref() == Some("login loop")
        })
        .await;

    rename(&transcript, "login redirect");
    pane.hook(&hook("Stop", &reading(&transcript))).await;
    client
        .until(&id, "the later title", |view| {
            view.title.as_deref() == Some("login redirect")
        })
        .await;

    rename(&transcript, "");
    pane.hook(&hook("Stop", &reading(&transcript))).await;
    client
        .until(&id, "the slug back", |view| view.title.is_none())
        .await;
}

#[tokio::test]
async fn desktop_notifications_name_the_session_by_its_title() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut client, "Ask me").await;
    pane.hook(&hook("UserPromptSubmit", &titled("login redirect")))
        .await;
    client
        .until(&id, "the title", |view| view.title.is_some())
        .await;

    pane.hook(&hook("PermissionRequest", r#""tool_name":"Bash""#))
        .await;

    let log = env
        .until_notified("a desktop notification", |log| {
            log.iter().any(|entry| entry["action"] == "show")
        })
        .await;
    let shown = log.iter().find(|entry| entry["action"] == "show").unwrap();
    assert_eq!(shown["title"], "login redirect");
}

#[tokio::test]
async fn a_rename_shows_without_waiting_for_the_next_hook() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut client, "Fix login").await;
    let transcript = env.path("transcript.jsonl");
    rename(&transcript, "login loop");
    pane.hook(&hook("Stop", &reading(&transcript))).await;
    client
        .until(&id, "the first title", |view| {
            view.title.as_deref() == Some("login loop")
        })
        .await;

    rename(&transcript, "login redirect");
    client
        .until(&id, "the title", |view| {
            view.title.as_deref() == Some("login redirect")
        })
        .await;
}
