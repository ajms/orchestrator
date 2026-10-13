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

#[tokio::test]
async fn a_sandbox_that_cannot_start_is_told_apart_from_an_agent_crash() {
    if !bwrap_runs() {
        return;
    }
    let sandbox = Sandbox::new();
    let missing = sandbox.path("gone");
    let policy = serde_json::to_string(&[
        "--ro-bind",
        "/",
        "/",
        "--bind",
        &missing.display().to_string(),
        "/mnt",
    ])
    .unwrap();
    let held = sandbox.hold_with("s1", "", &["--sandbox", &policy]);
    let (mut client, _) = held.attach().await;

    let exit = wait_for_exit(&mut client).await;
    let failure = exit.sandbox_failure.expect("a Sandbox failure");
    assert!(failure.contains("Can't find source path"), "{failure}");
}

#[tokio::test]
async fn an_agent_failing_inside_the_sandbox_is_not_a_sandbox_failure() {
    if !bwrap_runs() {
        return;
    }
    let sandbox = Sandbox::new();
    let held = sandbox.hold_with("s1", "exit 3\n", &["--sandbox", MINIMAL_POLICY]);
    let (mut client, _) = held.attach().await;

    let exit = wait_for_exit(&mut client).await;
    assert_eq!((exit.code, exit.sandbox_failure), (3, None));
}
