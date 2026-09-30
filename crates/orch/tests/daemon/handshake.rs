use orch_holder::{read_frame_async, write_frame_async};
use orch_protocol::{FromDaemon, PROTOCOL_VERSION, ToDaemon};
use tokio::net::UnixStream;

use crate::common::*;

#[tokio::test]
async fn a_client_is_welcomed_and_receives_the_full_session_list() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    assert!(client.session_list().await.is_empty());
}

#[tokio::test]
async fn a_client_speaking_another_protocol_version_is_told_so_explicitly() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut stream = UnixStream::connect(env.socket()).await.unwrap();
    let hello = ToDaemon::Hello {
        version: PROTOCOL_VERSION + 1,
        pane: None,
        display: None,
    };
    write_frame_async(&mut stream, &hello).await.unwrap();
    let reply: Option<FromDaemon> = read_frame_async(&mut stream).await.unwrap();
    match reply {
        Some(FromDaemon::VersionMismatch {
            daemon_version,
            message,
        }) => {
            assert_eq!(daemon_version, PROTOCOL_VERSION);
            assert!(message.contains("restart"), "{message}");
        }
        other => panic!("expected a version mismatch, got {other:?}"),
    }
    let closed: Option<FromDaemon> = read_frame_async(&mut stream).await.unwrap();
    assert_eq!(closed, None);
}

#[tokio::test]
async fn a_client_of_another_version_can_restart_the_daemon_and_sessions_survive() {
    let env = Env::new();
    let mut old = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Upgrade").await;
    let holder = client.holder_pid(&id);

    let mut stream = UnixStream::connect(env.socket()).await.unwrap();
    let hello = ToDaemon::Hello {
        version: PROTOCOL_VERSION + 1,
        pane: None,
        display: None,
    };
    write_frame_async(&mut stream, &hello).await.unwrap();
    let reply: Option<FromDaemon> = read_frame_async(&mut stream).await.unwrap();
    assert!(matches!(reply, Some(FromDaemon::VersionMismatch { .. })));

    let spawn =
        || orch_protocol::spawn_detached(env.daemon_command(std::time::Duration::from_secs(600)));
    let restarted = orch_protocol::restart_daemon(&env.runtime_dir(), spawn, WAIT)
        .await
        .unwrap();
    assert!(old.wait_exit().await.success());
    let mut client = TestClient::from(restarted);
    let listed = client.session_list().await;
    assert_eq!(listed[0].phase, orch_protocol::PhaseView::Active);
    assert_eq!(listed[0].holder_pid.map(|pid| pid as i32), holder);
}
