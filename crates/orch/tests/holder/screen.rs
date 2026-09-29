use orch_holder::{AgentStatus, FromHolder, HolderClient, PROTOCOL_VERSION, Size, ToHolder};

use crate::common::*;

#[tokio::test]
async fn holder_greets_the_daemon_and_serves_the_agent_screen() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "print hello from the agent\n");

    let (mut client, hello) = held.attach().await;

    assert_eq!(hello.version, PROTOCOL_VERSION);
    assert_eq!(hello.session.as_str(), "s1");
    assert_eq!(hello.holder_pid, held.pid as u32);
    assert_eq!(hello.agent, AgentStatus::Running);
    assert!(hello.agent_pid.is_some());
    wait_for_screen(&mut client, |text| text.contains("hello from the agent")).await;
}

#[tokio::test]
async fn input_bytes_reach_the_agent() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "");
    let (mut client, _) = held.attach().await;

    type_line(&mut client, "print typed by the user").await;

    wait_for_screen(&mut client, |text| text.contains("typed by the user")).await;
}

#[tokio::test]
async fn subscription_starts_with_a_snapshot_followed_by_incremental_output() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "print before\n");
    let (mut client, _) = held.attach().await;
    wait_for_screen(&mut client, |text| text.contains("before")).await;

    client.send(&ToHolder::Subscribe).await.unwrap();
    let mut mirror = loop {
        if let Some(FromHolder::Screen(snapshot)) = client.recv().await.unwrap() {
            break snapshot.restore(100);
        }
    };
    type_line(&mut client, "print after").await;
    while !mirror.screen().contents().contains("after") {
        match client.recv().await.unwrap().unwrap() {
            FromHolder::Output { bytes } => mirror.process(&bytes),
            _ => continue,
        }
    }

    let holder_view = client.snapshot().await.unwrap().text();
    assert_eq!(mirror.screen().contents(), holder_view);
    assert!(holder_view.contains("before"));
}

#[tokio::test]
async fn resize_changes_the_agent_terminal_and_the_emulator() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "");
    let (mut client, _) = held.attach().await;
    client.send(&ToHolder::Subscribe).await.unwrap();

    client
        .send(&ToHolder::Resize(Size {
            rows: 30,
            cols: 100,
        }))
        .await
        .unwrap();
    type_line(&mut client, "size").await;

    let snapshot = wait_for_screen(&mut client, |text| text.contains("30 100")).await;
    assert_eq!(
        snapshot.size,
        Size {
            rows: 30,
            cols: 100
        }
    );
    let mut resized = false;
    while let Ok(Ok(Some(message))) =
        tokio::time::timeout(std::time::Duration::from_millis(500), client.recv()).await
    {
        if message
            == FromHolder::Resized(Size {
                rows: 30,
                cols: 100,
            })
        {
            resized = true;
            break;
        }
    }
    assert!(resized, "subscribers are told about the new size");
}

#[tokio::test]
async fn paste_is_bracketed_when_the_agent_enabled_bracketed_paste() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "print \\e[?2004hbracketed on\n");
    let (mut client, _) = held.attach().await;
    let snapshot = wait_for_screen(&mut client, |text| text.contains("bracketed on")).await;
    assert!(snapshot.input_modes().bracketed_paste);

    client
        .send(&ToHolder::Paste {
            text: "print pasted".into(),
        })
        .await
        .unwrap();
    type_line(&mut client, "").await;

    wait_for_screen(&mut client, |text| {
        text.contains(r"unknown> \u{1b}[200~print pasted\u{1b}[201~")
    })
    .await;
}

#[tokio::test]
async fn paste_is_sent_verbatim_without_bracketed_paste() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "");
    let (mut client, _) = held.attach().await;

    client
        .send(&ToHolder::Paste {
            text: "print plain paste\r".into(),
        })
        .await
        .unwrap();

    wait_for_screen(&mut client, |text| text.contains("plain paste")).await;
}

#[tokio::test]
async fn snapshot_includes_scrollback() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "lines 60 row\nprint end of rows\n");
    let (mut client, _) = held.attach().await;

    let snapshot = wait_for_screen(&mut client, |text| text.contains("end of rows")).await;
    let mut restored = snapshot.restore(1000);
    restored.screen_mut().set_scrollback(usize::MAX);

    assert!(restored.screen().contents().starts_with("row 0\nrow 1\n"));
}

#[tokio::test]
async fn agent_environment_carries_session_worktree_and_port_block() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold_with(
        "s-env",
        "env ORCH_SESSION\nenv ORCH_WORKTREE\nenv ORCH_PORT_BASE\n",
        &[
            "--env",
            "ORCH_WORKTREE=/repo/wt",
            "--env",
            "ORCH_PORT_BASE=4100",
        ],
    );
    let (mut client, _) = held.attach().await;

    wait_for_screen(&mut client, |text| {
        text.contains("ORCH_SESSION=s-env")
            && text.contains("ORCH_WORKTREE=/repo/wt")
            && text.contains("ORCH_PORT_BASE=4100")
    })
    .await;
}

#[tokio::test]
async fn holder_runs_in_its_own_session_detached_from_the_caller() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "");

    let session_of = |pid: &str| {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        stat.rsplit(") ")
            .next()
            .unwrap()
            .split(' ')
            .nth(3)
            .unwrap()
            .to_string()
    };

    assert!(held.is_alive());
    assert_ne!(session_of(&held.pid.to_string()), session_of("self"));
}

#[tokio::test]
async fn holder_keeps_the_agent_alive_across_daemon_disconnects() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "print still here\n");
    let (client, _) = held.attach().await;
    drop(client);

    let (mut client, hello) = held.attach().await;

    assert_eq!(hello.agent, AgentStatus::Running);
    wait_for_screen(&mut client, |text| text.contains("still here")).await;
}

#[tokio::test]
async fn holder_keeps_the_final_screen_until_shutdown() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "print last words\nexit 3\n");
    let (mut client, _) = held.attach().await;
    wait_for_exit(&mut client).await;
    drop(client);

    let (mut client, hello) = held.attach().await;
    let AgentStatus::Exited(exit) = hello.agent else {
        panic!("agent should have exited: {hello:?}");
    };
    assert_eq!(exit.code, 3);
    let snapshot = client.snapshot().await.unwrap();
    assert!(snapshot.text().contains("last words"));
    assert!(held.is_alive());

    client.send(&ToHolder::Shutdown).await.unwrap();

    held.wait_gone().await;
    assert!(!held.socket.exists());
}

#[tokio::test]
async fn kill_stops_the_agent_but_not_the_holder() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "print running\n");
    let (mut client, _) = held.attach().await;
    wait_for_screen(&mut client, |text| text.contains("running")).await;

    client.send(&ToHolder::Kill).await.unwrap();

    let exit = wait_for_exit(&mut client).await;
    assert_eq!(exit.signal.as_deref(), Some("Hangup"));
    assert!(held.is_alive());
}

#[tokio::test]
async fn shutdown_of_a_running_agent_ends_agent_and_holder() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "");
    let (mut client, hello) = held.attach().await;
    let agent = hello.agent_pid.unwrap();

    client.send(&ToHolder::Shutdown).await.unwrap();

    held.wait_gone().await;
    assert!(!process_running(agent));
}

#[tokio::test]
async fn daemon_with_another_protocol_version_is_told_and_disconnected() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "");
    let mut client = HolderClient::connect(&held.socket).await.unwrap();

    client
        .send(&ToHolder::Attach { version: 999 })
        .await
        .unwrap();

    let Some(FromHolder::Hello(hello)) = client.recv().await.unwrap() else {
        panic!("expected hello");
    };
    assert_eq!(hello.version, PROTOCOL_VERSION);
    assert_eq!(client.recv().await.unwrap(), None);
}

#[tokio::test]
async fn second_holder_for_the_same_session_is_refused() {
    let sandbox = Sandbox::new();
    let _held = sandbox.hold("s1", "");

    let output = sandbox
        .orch()
        .args(["hold", "--session", "s1", "--runtime-dir"])
        .arg(sandbox.runtime_dir())
        .args(["--", "true"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("already"));
}

#[tokio::test]
async fn alternate_screen_mirror_stays_in_sync_and_returns_to_main_history() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold(
        "s1",
        "lines 40 row\nprint \\e[?1049h\\e[Hin the full screen view\n",
    );
    let (mut client, _) = held.attach().await;
    wait_for_screen(&mut client, |text| text.contains("full screen view")).await;

    client.send(&ToHolder::Subscribe).await.unwrap();
    let (snapshot, mut mirror) = loop {
        if let Some(FromHolder::Screen(snapshot)) = client.recv().await.unwrap() {
            let mirror = snapshot.restore(1000);
            break (snapshot, mirror);
        }
    };
    assert!(snapshot.alternate_screen());
    assert_eq!(mirror.screen().contents(), "in the full screen view");

    type_line(&mut client, "print \\e[?1049lback on main").await;
    while !mirror.screen().contents().contains("back on main") {
        if let FromHolder::Output { bytes } = client.recv().await.unwrap().unwrap() {
            mirror.process(&bytes);
        }
    }

    assert!(!mirror.screen().alternate_screen());
    assert_eq!(
        mirror.screen().contents(),
        client.snapshot().await.unwrap().text()
    );
    mirror.screen_mut().set_scrollback(usize::MAX);
    assert!(mirror.screen().contents().starts_with("row 0\nrow 1\n"));
}

#[tokio::test]
async fn kill_escalates_to_sigkill_when_the_agent_ignores_hangup() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "ignore-hangup\n");
    let (mut client, _) = held.attach().await;
    wait_for_screen(&mut client, |text| text.contains("hangup ignored")).await;

    client.send(&ToHolder::Kill).await.unwrap();

    let exit = wait_for_exit(&mut client).await;
    assert_eq!(exit.signal.as_deref(), Some("Killed"));
    assert!(held.is_alive());
}

#[tokio::test]
async fn shutdown_leaves_no_agent_behind_even_if_it_ignores_hangup() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "ignore-hangup\n");
    let (mut client, hello) = held.attach().await;
    wait_for_screen(&mut client, |text| text.contains("hangup ignored")).await;
    let agent = hello.agent_pid.unwrap();

    client.send(&ToHolder::Shutdown).await.unwrap();

    held.wait_gone().await;
    assert!(!process_running(agent));
}

#[tokio::test]
async fn a_new_daemon_supersedes_the_previous_one() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "");
    let (mut previous, _) = held.attach().await;
    while let Ok(Ok(Some(_))) =
        tokio::time::timeout(std::time::Duration::from_millis(200), previous.recv()).await
    {}

    let (_current, _) = held.attach().await;

    for expected in [Some(FromHolder::Superseded), None] {
        let message = tokio::time::timeout(WAIT, previous.recv())
            .await
            .expect("previous daemon left hanging")
            .unwrap();
        assert_eq!(message, expected);
    }
}
