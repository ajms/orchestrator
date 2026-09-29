use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::config::Display;
use crate::event::Effect;

pub fn copy_effects(display: &Display, text: &str) -> Vec<Effect> {
    match (&display.wayland, &display.x11) {
        (Some(_), _) => vec![
            command(text, "wl-copy", &[]),
            command(text, "wl-copy", &["--primary"]),
        ],
        (None, Some(_)) => vec![
            command(text, "xclip", &["-selection", "clipboard"]),
            command(text, "xclip", &["-selection", "primary"]),
        ],
        (None, None) => vec![
            Effect::WriteTerminal(osc52('c', text)),
            Effect::WriteTerminal(osc52('p', text)),
        ],
    }
}

fn command(text: &str, program: &str, args: &[&str]) -> Effect {
    Effect::CopyCommand {
        program: program.into(),
        args: args.iter().map(|arg| arg.to_string()).collect(),
        text: text.into(),
    }
}

fn osc52(target: char, text: &str) -> Vec<u8> {
    format!("\x1b]52;{target};{}\x07", STANDARD.encode(text)).into_bytes()
}
