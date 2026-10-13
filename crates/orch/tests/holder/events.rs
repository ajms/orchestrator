use orch_holder::{HolderClient, HolderEvent, ToHolder};

use crate::common::*;

const SESSION_START: &str =
    r#"{"hook_event_name":"SessionStart","session_id":"c1","source":"startup"}"#;
const STOP: &str = r#"{"hook_event_name":"Stop","session_id":"c1"}"#;
const STATUS: &str = r#"{"model":{"display_name":"Opus"},"context_window":{"used_percentage":42}}"#;

fn hook(payload: &str) -> HolderEvent {
    HolderEvent::Hook {
        payload: payload.into(),
        guard: None,
    }
}

fn tap(payload: &str) -> HolderEvent {
    HolderEvent::Tap {
        payload: payload.into(),
    }
}

async fn watch_screen(held: &Held, predicate: impl Fn(&str) -> bool) {
    let mut observer = HolderClient::connect(&held.socket).await.unwrap();
    wait_for_screen(&mut observer, predicate).await;
}

#[tokio::test]
async fn events_raised_before_the_daemon_connects_are_delivered_in_order() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold(
        "s1",
        &format!("hook {SESSION_START}\ntap {STATUS}\nhook {STOP}\nprint done\n"),
    );
    watch_screen(&held, |text| text.contains("done")).await;

    let (mut client, _) = held.attach().await;

    let mut events = Vec::new();
    for _ in 0..4 {
        events.push(next_event(&mut client).await);
    }
    let seqs: Vec<u64> = events.iter().map(|(seq, _)| *seq).collect();
    assert!(seqs.windows(2).all(|pair| pair[0] < pair[1]), "{seqs:?}");
    let kinds: Vec<HolderEvent> = events.into_iter().map(|(_, event)| event).collect();
    assert!(matches!(kinds[0], HolderEvent::Spawned { pid: Some(_) }));
    assert_eq!(kinds[1..], [hook(SESSION_START), tap(STATUS), hook(STOP)]);
}

#[tokio::test]
async fn events_raised_while_attached_are_forwarded_live() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "");
    let (mut client, _) = held.attach().await;

    type_line(&mut client, &format!("hook {STOP}")).await;

    assert_eq!(next_hook_or_tap(&mut client).await, hook(STOP));
}

#[tokio::test]
async fn unacknowledged_events_are_replayed_to_the_next_daemon() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold(
        "s1",
        &format!("hook {SESSION_START}\nhook {STOP}\nprint done\n"),
    );
    watch_screen(&held, |text| text.contains("done")).await;

    let (mut first, _) = held.attach().await;
    let (spawned_seq, _) = next_event(&mut first).await;
    first
        .send(&ToHolder::Ack {
            through: spawned_seq + 1,
        })
        .await
        .unwrap();
    first.snapshot().await.unwrap();
    drop(first);

    let (mut second, _) = held.attach().await;

    assert_eq!(next_event(&mut second).await, (spawned_seq + 2, hook(STOP)));
}

#[tokio::test]
async fn agent_exit_is_reported_with_its_code() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "exit 7\n");
    let (mut client, _) = held.attach().await;

    let exit = wait_for_exit(&mut client).await;

    assert_eq!(exit.code, 7);
    assert_eq!(exit.signal, None);
}

#[tokio::test]
async fn event_buffer_is_bounded_and_keeps_the_newest_events() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold_with(
        "s1",
        &format!("hook {SESSION_START}\ntap {STATUS}\nhook {STOP}\nprint done\n"),
        &["--event-capacity", "2"],
    );
    watch_screen(&held, |text| text.contains("done")).await;

    let (mut client, _) = held.attach().await;

    assert_eq!(next_event(&mut client).await.1, tap(STATUS));
    assert_eq!(next_event(&mut client).await.1, hook(STOP));
}

#[tokio::test]
async fn statusline_samples_are_coalesced_so_they_cannot_push_out_hook_events() {
    let sandbox = Sandbox::new();
    let sample = |n: u32| {
        format!(
            r#"{{"model":{{"display_name":"Opus"}},"context_window":{{"used_percentage":{n}}}}}"#
        )
    };
    let script = format!(
        "hook {SESSION_START}\ntap {}\ntap {}\ntap {}\nhook {STOP}\nprint done\n",
        sample(1),
        sample(2),
        sample(3)
    );
    let held = sandbox.hold_with("s1", &script, &["--event-capacity", "3"]);
    watch_screen(&held, |text| text.contains("done")).await;

    let (mut client, _) = held.attach().await;

    assert_eq!(next_event(&mut client).await.1, hook(SESSION_START));
    assert_eq!(next_event(&mut client).await.1, tap(&sample(3)));
    assert_eq!(next_event(&mut client).await.1, hook(STOP));
}

#[tokio::test]
async fn guard_requests_get_through_while_the_agent_floods_a_slow_subscriber() {
    use orch_holder::FromHolder;
    const PRE_TOOL_USE: &str =
        r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"ls"}}"#;

    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "flood\n");
    let (mut client, _) = held.attach().await;
    client.send(&ToHolder::Subscribe).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    let mut hook = sandbox.orch();
    hook.args(["hook", "--session", "s1"])
        .env(orch_holder::HOLDER_SOCKET_ENV, &held.socket);
    let guarded = std::thread::spawn(move || run_with_stdin(&mut hook, PRE_TOOL_USE));

    let started = std::time::Instant::now();
    let mut output_bytes = 0;
    let id = loop {
        assert!(
            started.elapsed() < WAIT,
            "guard request stuck behind output"
        );
        match client.recv().await.unwrap().expect("holder closed") {
            FromHolder::Output { bytes } => output_bytes += bytes.len(),
            FromHolder::Event {
                event: HolderEvent::Hook {
                    guard: Some(id), ..
                },
                ..
            } => break id,
            _ => {}
        }
    };
    client
        .send(&ToHolder::GuardAnswer {
            id,
            answer: orch_agent::GuardAnswer::Deny {
                reason: "busy".into(),
            },
        })
        .await
        .unwrap();

    let answered = guarded.join().unwrap();
    assert!(String::from_utf8_lossy(&answered.stdout).contains("deny"));
    assert!(
        output_bytes < 8 << 20,
        "{output_bytes} bytes of output before the guard"
    );
    assert!(holder_memory_kb(held.pid) < 200_000);
}

#[tokio::test]
async fn a_subscriber_that_falls_behind_gets_a_fresh_snapshot() {
    use orch_holder::FromHolder;

    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "flood\n");
    let (mut client, _) = held.attach().await;
    client.send(&ToHolder::Subscribe).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    let started = std::time::Instant::now();
    let mut snapshots = 0;
    while snapshots < 2 {
        assert!(started.elapsed() < WAIT, "no resync snapshot");
        if let Some(FromHolder::Screen(snapshot)) = client.recv().await.unwrap() {
            snapshots += 1;
            assert!(snapshots == 1 || snapshot.text().contains("flood"));
        }
    }
    assert!(holder_memory_kb(held.pid) < 200_000);
}
