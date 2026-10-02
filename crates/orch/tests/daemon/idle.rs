use std::time::Duration;

use orch_protocol::{connect_or_spawn, spawn_detached};

use crate::common::*;

const IDLE: Duration = Duration::from_millis(300);

async fn stays_up(daemon: &mut Daemon) {
    tokio::time::sleep(IDLE * 4).await;
    assert!(!daemon.has_exited(), "the Daemon exited while still needed");
}

#[tokio::test]
async fn the_daemon_exits_once_idle_without_clients_or_holders() {
    let env = Env::new();
    let mut daemon = env.start_daemon_with(IDLE).await;
    let client = env.client().await;
    stays_up(&mut daemon).await;

    drop(client);
    assert!(daemon.wait_exit().await.success());
    assert!(!env.socket().exists());
}

#[tokio::test]
async fn a_live_holder_keeps_the_daemon_running() {
    let env = Env::new();
    let mut daemon = env.start_daemon_with(IDLE).await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Keep alive").await;
    let holder = client.holder_pid(&id).unwrap();
    drop(client);
    stays_up(&mut daemon).await;

    kill(holder, "-KILL");
    assert!(daemon.wait_exit().await.success());
}

#[tokio::test]
async fn a_client_spawns_the_daemon_when_none_is_running() {
    let env = Env::new();
    let spawn = || spawn_detached(env.daemon_command(Duration::from_secs(600)));
    let mut client = connect_or_spawn(&env.socket(), spawn, WAIT).await.unwrap();
    let first = client.recv().await.unwrap();
    assert!(matches!(
        first,
        Some(orch_protocol::FromDaemon::Sessions { .. })
    ));

    let unused = || -> std::io::Result<()> { panic!("a running Daemon is reused") };
    connect_or_spawn(&env.socket(), unused, WAIT).await.unwrap();
}
