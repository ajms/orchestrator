use orch_protocol::{AgentStateView as State, DisplayVars, Request};

use crate::common::*;

fn display(wayland: Option<&str>, x11: Option<&str>) -> DisplayVars {
    DisplayVars {
        wayland_display: wayland.map(Into::into),
        x11_display: x11.map(Into::into),
    }
}

#[tokio::test]
async fn a_new_agent_gets_the_display_variables_of_the_latest_client() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let _early = env.tui(display(Some("wayland-early"), Some(":7"))).await;
    let _late = env.tui(display(Some("wayland-late"), None)).await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Copy things").await;

    let mut pane = env.pane(&id, PANE).await;
    pane.type_line("env WAYLAND_DISPLAY").await;
    pane.type_line("env DISPLAY").await;
    pane.wait_for_text("WAYLAND_DISPLAY=wayland-late").await;
    pane.wait_for_text("DISPLAY=<unset>").await;
}

#[tokio::test]
async fn a_one_shot_cli_connection_leaves_the_tuis_display_in_place() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let _tui = env.tui(display(Some("wayland-tui"), Some(":3"))).await;
    let mut cli = env.client().await;
    let id = running_session(&env, &mut cli, "From the CLI").await;

    let mut pane = env.pane(&id, PANE).await;
    pane.type_line("env WAYLAND_DISPLAY").await;
    pane.type_line("env DISPLAY").await;
    pane.wait_for_text("WAYLAND_DISPLAY=wayland-tui").await;
    pane.wait_for_text("DISPLAY=:3").await;
}

#[tokio::test]
async fn a_tui_without_a_display_leaves_the_earlier_display_in_place() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let _desktop = env.tui(display(Some("wayland-desk"), Some(":5"))).await;
    let _ssh = env.tui(DisplayVars::default()).await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Over ssh").await;

    let mut pane = env.pane(&id, PANE).await;
    pane.type_line("env WAYLAND_DISPLAY").await;
    pane.type_line("env DISPLAY").await;
    pane.wait_for_text("WAYLAND_DISPLAY=wayland-desk").await;
    pane.wait_for_text("DISPLAY=:5").await;
}

#[tokio::test]
async fn an_agent_gets_no_display_variables_when_no_client_reported_any() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Headless").await;

    let mut pane = env.pane(&id, PANE).await;
    pane.type_line("env WAYLAND_DISPLAY").await;
    pane.wait_for_text("WAYLAND_DISPLAY=<unset>").await;
}

#[tokio::test]
async fn a_resumed_agent_gets_the_display_of_the_latest_client_while_a_running_one_keeps_its_own() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let _first = env.tui(display(None, Some(":1"))).await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Resume me").await;
    let mut pane = env.pane(&id, PANE).await;
    pane.hook(&hook("SessionStart", "")).await;

    let _later = env.tui(display(None, Some(":2"))).await;
    pane.type_line("env DISPLAY").await;
    pane.wait_for_text("DISPLAY=:1").await;
    pane.type_line("exit 0").await;
    client
        .until(&id, "Exited", |view| view.agent == Some(State::Exited))
        .await;

    client
        .request(Request::Resume {
            session: id.clone(),
        })
        .await
        .unwrap();
    client
        .until(&id, "running again", |view| {
            matches!(view.agent, Some(State::Starting | State::Working))
        })
        .await;
    let mut resumed = env.pane(&id, PANE).await;
    resumed.type_line("env DISPLAY").await;
    resumed.wait_for_text("DISPLAY=:2").await;
}
