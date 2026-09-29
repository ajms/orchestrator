use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use orch_protocol::{DisplayVars, FromDaemon};
use orch_tui::{Effect, TuiConfig};

use crate::common::*;

fn yank_in(display: DisplayVars) -> Vec<Effect> {
    let mut tui = Harness::with_config(TuiConfig {
        display,
        ..TuiConfig::default()
    });
    tui.sessions(vec![session("webshop", "first")]);
    tui.pane(
        "first",
        FromDaemon::Output {
            bytes: b"alpha\r\nbeta".to_vec(),
        },
    );
    tui.keys("l");
    tui.take_effects();
    tui.keys("Vky");
    tui.take_effects()
}

fn command(program: &str, args: &[&str]) -> Effect {
    Effect::CopyCommand {
        program: program.into(),
        args: args.iter().map(|arg| arg.to_string()).collect(),
        text: "alpha\nbeta".into(),
    }
}

fn osc52(target: char) -> Effect {
    let payload = STANDARD.encode("alpha\nbeta");
    Effect::WriteTerminal(format!("\x1b]52;{target};{payload}\x07").into_bytes())
}

#[test]
fn under_wayland_a_yank_runs_wl_copy_for_the_clipboard_and_primary() {
    let effects = yank_in(DisplayVars {
        wayland_display: Some("wayland-0".into()),
        x11_display: Some(":0".into()),
    });

    assert_eq!(
        effects,
        vec![command("wl-copy", &[]), command("wl-copy", &["--primary"])]
    );
}

#[test]
fn under_x11_a_yank_runs_xclip_for_the_clipboard_and_primary() {
    let effects = yank_in(DisplayVars {
        wayland_display: None,
        x11_display: Some(":0".into()),
    });

    assert_eq!(
        effects,
        vec![
            command("xclip", &["-selection", "clipboard"]),
            command("xclip", &["-selection", "primary"])
        ]
    );
}

#[test]
fn without_a_display_a_yank_writes_osc_52_for_the_clipboard_and_primary() {
    let effects = yank_in(DisplayVars::default());

    assert_eq!(effects, vec![osc52('c'), osc52('p')]);
}
