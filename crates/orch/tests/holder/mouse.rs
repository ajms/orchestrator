use crate::common::*;

#[tokio::test]
async fn the_agents_mouse_request_is_carried_in_the_screen_snapshot() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "mouse button-motion sgr\nprint mouse on\n");
    let (mut client, _) = held.attach().await;

    let snapshot = wait_for_screen(&mut client, |text| text.contains("mouse on")).await;

    let mirror = snapshot.restore(0);
    assert_eq!(
        mirror.screen().mouse_protocol_mode(),
        vt100::MouseProtocolMode::ButtonMotion
    );
    assert_eq!(
        mirror.screen().mouse_protocol_encoding(),
        vt100::MouseProtocolEncoding::Sgr
    );
}

#[tokio::test]
async fn forwarded_mouse_bytes_reach_the_agent_intact() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "mouse press-release sgr\nraw\n");
    let (mut client, _) = held.attach().await;
    wait_for_screen(&mut client, |text| text.contains("raw on")).await;

    send_bytes(&mut client, b"\x1b[<0;12;5M").await;
    wait_for_screen(&mut client, |text| text.contains(r"raw> \x1b[<0;12;5M")).await;
    send_bytes(&mut client, b"\x1b[<0;12;5m").await;

    wait_for_screen(&mut client, |text| text.contains(r"raw> \x1b[<0;12;5m")).await;
}

#[tokio::test]
async fn the_agent_leaves_raw_mode_on_ctrl_d_and_takes_commands_again() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "raw\n");
    let (mut client, _) = held.attach().await;
    wait_for_screen(&mut client, |text| text.contains("raw on")).await;

    send_bytes(&mut client, b"\x04").await;
    wait_for_screen(&mut client, |text| text.contains("raw off")).await;
    type_line(&mut client, "print back to lines").await;

    wait_for_screen(&mut client, |text| text.contains("back to lines")).await;
}
