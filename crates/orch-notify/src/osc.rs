pub fn terminal_attention(title: &str, body: &str) -> Vec<u8> {
    let title: String = sanitise(title)
        .map(|c| if c == ';' { ',' } else { c })
        .collect();
    let body: String = sanitise(body).collect();
    format!("\x1b]777;notify;{title};{body}\x07\x07").into_bytes()
}

fn sanitise(text: &str) -> impl Iterator<Item = char> {
    text.chars().map(|c| if c.is_control() { ' ' } else { c })
}
