use orch_core::SessionId;
use orch_protocol::Request;
use serde_json::Value;

use crate::common::*;

fn shown(notifications: &[Value]) -> Vec<&Value> {
    notifications
        .iter()
        .filter(|entry| entry["action"] == "show")
        .collect()
}

#[tokio::test]
async fn a_session_needing_input_unwatched_notifies_the_desktop_and_rings_the_client() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut client, "Ask me").await;

    pane.hook(&hook("PermissionRequest", r#""tool_name":"Bash""#))
        .await;

    let log = env
        .until_notified("a desktop notification", |log| !shown(log).is_empty())
        .await;
    let notification = shown(&log)[0];
    assert_eq!(notification["key"], id.as_str());
    assert_eq!(notification["focus"], id.as_str());
    let branch = client.sessions[&id].branch.clone();
    assert_eq!(notification["body"], format!("Needs input · app/{branch}"));
    client
        .until_received("a Ring", |client| !client.rings.is_empty())
        .await;
    assert_eq!(client.rings[0].session, id);
    assert!(client.rings[0].body.starts_with("Needs input"));
}

fn about<'a>(notifications: &'a [Value], id: &SessionId) -> Vec<&'a Value> {
    shown(notifications)
        .into_iter()
        .filter(|entry| entry["key"] == id.as_str())
        .collect()
}

fn says(entry: &Value, text: &str) -> bool {
    entry["body"]
        .as_str()
        .is_some_and(|body| body.contains(text))
}

#[tokio::test]
async fn nothing_is_sent_for_a_session_a_focused_client_is_looking_at() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut client, "Watched").await;

    client.view(Some(&id), true).await;
    pane.hook(&hook("PermissionRequest", r#""tool_name":"Bash""#))
        .await;
    client
        .until(&id, "Needs input", |view| {
            view.agent_state == Some(orch_protocol::AgentStateView::NeedsInput)
        })
        .await;
    client.view(Some(&id), false).await;
    pane.hook(&hook("Stop", "")).await;

    let log = env
        .until_notified("the unfocused turn end", |log| {
            about(log, &id).iter().any(|entry| says(entry, "Finished"))
        })
        .await;
    assert!(
        !about(&log, &id)
            .iter()
            .any(|entry| says(entry, "Needs input")),
        "{log:?}"
    );
    client.drain().await;
    assert!(
        !client
            .rings
            .iter()
            .any(|ring| ring.body.contains("Needs input")),
        "{:?}",
        client.rings
    );
}

fn closed(notifications: &[Value], key: &str) -> bool {
    notifications
        .iter()
        .any(|entry| entry["action"] == "close" && entry["key"] == key)
}

async fn needs_input(client: &mut TestClient, id: &SessionId, pane: &mut PaneView) {
    pane.hook(&hook("PermissionRequest", r#""tool_name":"Bash""#))
        .await;
    client
        .until(id, "Needs input", |view| {
            view.agent_state == Some(orch_protocol::AgentStateView::NeedsInput)
        })
        .await;
}

#[tokio::test]
async fn a_sessions_notification_is_replaced_in_place_by_its_next_event() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut client, "Twice").await;

    needs_input(&mut client, &id, &mut pane).await;
    pane.hook(&hook("Stop", "")).await;

    let log = env
        .until_notified("the second notification", |log| about(log, &id).len() == 2)
        .await;
    assert!(says(about(&log, &id)[0], "Needs input"));
    assert!(says(about(&log, &id)[1], "Finished"));
    assert!(shown(&log).iter().all(|entry| entry["key"] == id.as_str()));
    assert!(!closed(&log, id.as_str()), "{log:?}");
}

#[tokio::test]
async fn sessions_calling_within_a_few_seconds_are_merged_into_one_summary() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (first, mut first_pane) = idle_session(&env, &mut client, "First").await;
    let (second, mut second_pane) = idle_session(&env, &mut client, "Second").await;

    needs_input(&mut client, &first, &mut first_pane).await;
    needs_input(&mut client, &second, &mut second_pane).await;

    let log = env
        .until_notified("a merged summary", |log| {
            shown(log).iter().any(|entry| entry["key"] == "merged")
        })
        .await;
    assert!(closed(&log, first.as_str()), "{log:?}");
    let merged = shown(&log)
        .into_iter()
        .find(|entry| entry["key"] == "merged");
    let merged = merged.unwrap();
    assert_eq!(merged["title"], "2 Sessions need you");
    assert_eq!(merged["focus"], second.as_str());
}

#[tokio::test]
async fn a_muted_session_neither_notifies_nor_rings_and_muting_dismisses_its_notification() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (muted, mut muted_pane) = idle_session(&env, &mut client, "Quiet").await;
    let (loud, mut loud_pane) = idle_session(&env, &mut client, "Loud").await;

    needs_input(&mut client, &muted, &mut muted_pane).await;
    env.until_notified("the first call", |log| !about(log, &muted).is_empty())
        .await;
    client
        .request(Request::SetMuted {
            session: muted.clone(),
            muted: true,
        })
        .await
        .unwrap();
    env.until_notified("the dismissal", |log| closed(log, muted.as_str()))
        .await;

    muted_pane.hook(&hook("Stop", "")).await;
    client
        .until(&muted, "Idle", |view| {
            view.agent_state == Some(orch_protocol::AgentStateView::Idle)
        })
        .await;
    needs_input(&mut client, &loud, &mut loud_pane).await;
    let log = env
        .until_notified("the unmuted call", |log| !about(log, &loud).is_empty())
        .await;
    assert_eq!(about(&log, &muted).len(), 1, "{log:?}");
    client.drain().await;
    assert!(
        client
            .rings
            .iter()
            .filter(|ring| ring.session == muted)
            .count()
            == 1,
        "{:?}",
        client.rings
    );
}

#[tokio::test]
async fn a_repo_override_turns_a_trigger_off_and_is_read_at_use() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut client, "Configured").await;
    let repo = env.path("repos/app");
    env.write_config(&format!(
        "[repos.{:?}.notifications.desktop]\nneeds_input = false\n",
        repo.display().to_string()
    ));

    needs_input(&mut client, &id, &mut pane).await;
    client
        .until_received("the Ring", |client| !client.rings.is_empty())
        .await;
    pane.hook(&hook("Stop", "")).await;
    let log = env
        .until_notified("the turn end", |log| !about(log, &id).is_empty())
        .await;
    assert_eq!(about(&log, &id).len(), 1, "{log:?}");
    assert!(says(about(&log, &id)[0], "Finished"));
}

#[tokio::test]
async fn looking_at_a_session_or_discarding_it_dismisses_its_notification() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (viewed, mut viewed_pane) = idle_session(&env, &mut client, "Viewed").await;

    needs_input(&mut client, &viewed, &mut viewed_pane).await;
    env.until_notified("the call", |log| !about(log, &viewed).is_empty())
        .await;
    client.view(Some(&viewed), true).await;
    env.until_notified("the dismissal", |log| closed(log, viewed.as_str()))
        .await;

    client.view(None, false).await;
    let (discarded, mut discarded_pane) = idle_session(&env, &mut client, "Discarded").await;
    needs_input(&mut client, &discarded, &mut discarded_pane).await;
    env.until_notified("the call", |log| !about(log, &discarded).is_empty())
        .await;
    client
        .request(Request::Discard {
            session: discarded.clone(),
            skip_teardown: false,
        })
        .await
        .unwrap();
    env.until_notified("the dismissal", |log| closed(log, discarded.as_str()))
        .await;
}

#[tokio::test]
async fn clicking_a_notification_focuses_the_session_in_the_most_recently_used_client() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut used = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut used, "Click me").await;
    let mut newer = env.client().await;
    newer.drain().await;
    needs_input(&mut used, &id, &mut pane).await;
    env.until_notified("the call", |log| !about(log, &id).is_empty())
        .await;
    used.view(None, false).await;

    env.click_notification(id.as_str());

    used.until_received("Focus", |client| !client.focused.is_empty())
        .await;
    assert_eq!(used.focused, [id]);
    newer.drain().await;
    assert!(newer.focused.is_empty());
}

#[tokio::test]
async fn desktop_notifications_arrive_with_no_client_connected() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut client, "Alone").await;
    drop(client);

    pane.hook(&hook("PermissionRequest", r#""tool_name":"Bash""#))
        .await;
    env.until_notified("the call", |log| !about(log, &id).is_empty())
        .await;
}

#[tokio::test]
async fn typing_in_a_clients_pane_makes_it_the_most_recently_used_client() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut typing = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut typing, "Type here").await;
    typing.view(Some(&id), false).await;
    needs_input(&mut typing, &id, &mut pane).await;
    env.until_notified("the call", |log| !about(log, &id).is_empty())
        .await;
    let mut other = env.client().await;
    other.view(None, false).await;

    pane.type_line("print typed").await;
    pane.wait_for_text("typed").await;
    env.click_notification(id.as_str());

    typing
        .until_received("Focus", |client| !client.focused.is_empty())
        .await;
    other.drain().await;
    assert!(other.focused.is_empty());
}

#[tokio::test]
async fn a_restarted_daemon_and_reconciliation_do_not_replay_old_attention() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut client, "Remembered").await;
    needs_input(&mut client, &id, &mut pane).await;
    env.until_notified("the call", |log| !about(log, &id).is_empty())
        .await;

    daemon.kill();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    client
        .until(&id, "adopted", |view| {
            view.agent_state == Some(orch_protocol::AgentStateView::NeedsInput)
        })
        .await;
    client.reconcile().await;
    client.view(Some(&id), false).await;

    let mut pane = env.pane(&id, PANE).await;
    pane.hook(&hook("Stop", "")).await;
    let log = env
        .until_notified("the next turn end", |log| {
            about(log, &id).iter().any(|entry| says(entry, "Finished"))
        })
        .await;
    let calls: Vec<_> = about(&log, &id)
        .iter()
        .map(|entry| entry["body"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(calls[0].starts_with("Needs input"));
    client.drain().await;
    assert!(
        client
            .rings
            .iter()
            .all(|ring| ring.body.starts_with("Finished")),
        "{:?}",
        client.rings
    );
}

async fn hold_replaying(
    socket: std::path::PathBuf,
    session: SessionId,
    holder_pid: u32,
    events: Vec<(u64, String)>,
) {
    use orch_holder::{
        AgentStatus, FromHolder, Hello, HolderEvent, PROTOCOL_VERSION, ToHolder, read_frame_async,
        write_frame_async,
    };
    let _ = std::fs::remove_file(&socket);
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let (session, events) = (session.clone(), events.clone());
            tokio::spawn(async move {
                let (mut reader, mut writer) = stream.into_split();
                while let Ok(Some(message)) = read_frame_async::<ToHolder>(&mut reader).await {
                    if !matches!(message, ToHolder::Attach { .. }) {
                        continue;
                    }
                    let hello = Hello {
                        version: PROTOCOL_VERSION,
                        session: session.clone(),
                        cwd: None,
                        base: None,
                        port_block: None,
                        holder_pid,
                        agent_pid: None,
                        agent: AgentStatus::Running,
                        sandboxed: false,
                    };
                    let _ = write_frame_async(&mut writer, &FromHolder::Hello(hello)).await;
                    for (seq, payload) in &events {
                        let event = HolderEvent::Hook {
                            payload: payload.clone(),
                            guard: None,
                            agent: "claude".into(),
                        };
                        let event = FromHolder::Event { seq: *seq, event };
                        let _ = write_frame_async(&mut writer, &event).await;
                    }
                }
            });
        }
    });
}

#[tokio::test]
async fn a_restarted_daemon_skips_the_holder_events_it_already_applied() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, mut pane) = idle_session(&env, &mut client, "Replayed").await;
    needs_input(&mut client, &id, &mut pane).await;
    env.until_notified("the call", |log| !about(log, &id).is_empty())
        .await;
    let holder = client.holder_pid(&id).unwrap();
    daemon.kill();
    kill(holder, "-KILL");
    wait_until("the Holder is gone", || process_gone(holder)).await;

    let mut stand_in = std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .unwrap();
    let applied = vec![
        (1, hook("Stop", "")),
        (2, hook("PermissionRequest", r#""tool_name":"Bash""#)),
    ];
    let unseen = (1000, hook("StopFailure", r#""error":"server_error""#));
    hold_replaying(
        env.holder_socket(&id),
        id.clone(),
        stand_in.id(),
        [applied, vec![unseen]].concat(),
    )
    .await;
    let _daemon = env.start_daemon().await;

    let log = env
        .until_notified("the unseen event", |log| {
            about(log, &id).iter().any(|entry| says(entry, "Errored"))
        })
        .await;
    let _ = stand_in.kill();
    let _ = stand_in.wait();
    let calls: Vec<_> = about(&log, &id)
        .iter()
        .map(|entry| entry["body"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(calls[0].starts_with("Needs input"));
    assert!(calls[1].starts_with("Errored"));
}
