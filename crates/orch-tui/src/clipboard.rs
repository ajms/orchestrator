use base64::Engine;
use base64::engine::general_purpose::STANDARD;

pub fn osc52(text: &str) -> Vec<u8> {
    format!("\x1b]52;c;{}\x07", STANDARD.encode(text)).into_bytes()
}
