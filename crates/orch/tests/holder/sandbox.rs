use orch_holder::ToHolder;

use crate::common::*;

const MINIMAL_POLICY: &str = r#"["--ro-bind","/","/","--dev","/dev","--proc","/proc"]"#;

fn bwrap_runs() -> bool {
    let runs = std::process::Command::new("bwrap")
        .args([
            "--ro-bind",
            "/",
            "/",
            "--dev",
            "/dev",
            "--proc",
            "/proc",
            "true",
        ])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !runs {
        eprintln!("skipped: bubblewrap cannot run here");
    }
    runs
}

#[tokio::test]
async fn a_sandboxed_agent_says_so_and_still_reaches_its_holder() {
    if !bwrap_runs() {
        return;
    }
    let sandbox = Sandbox::new();
    let held = sandbox.hold_with(
        "s1",
        "print boxed\nhook {\"hook_event_name\":\"Stop\"}\n",
        &["--sandbox", MINIMAL_POLICY],
    );
    let (mut client, hello) = held.attach().await;
    assert!(hello.sandboxed);

    let event = next_hook_or_tap(&mut client).await;
    assert!(matches!(event, orch_holder::HolderEvent::Hook { .. }));
    wait_for_screen(&mut client, |text| text.contains("boxed")).await;
}

#[tokio::test]
async fn stopping_a_sandboxed_agent_lets_it_exit_cleanly() {
    if !bwrap_runs() {
        return;
    }
    let sandbox = Sandbox::new();
    let held = sandbox.hold_with("s1", "exit-on-hangup\n", &["--sandbox", MINIMAL_POLICY]);
    let (mut client, _) = held.attach().await;
    wait_for_screen(&mut client, |text| text.contains("hangup handled")).await;

    client.send(&ToHolder::Kill).await.unwrap();

    let exit = wait_for_exit(&mut client).await;
    assert_eq!((exit.code, exit.signal), (0, None));
}

#[tokio::test]
async fn an_unsandboxed_agent_says_so() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "");
    let (_, hello) = held.attach().await;
    assert!(!hello.sandboxed);
}
