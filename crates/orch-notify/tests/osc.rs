use orch_notify::terminal_attention;

#[test]
fn encodes_an_osc_777_notification_followed_by_a_bell() {
    assert_eq!(
        terminal_attention("fix-login", "Needs input"),
        b"\x1b]777;notify;fix-login;Needs input\x07\x07".to_vec()
    );
}

#[test]
fn control_characters_cannot_escape_the_sequence() {
    let encoded = terminal_attention("evil\x1b]0;pwned\x07", "line one\nline two\u{9b}2J\r");
    assert_eq!(
        encoded,
        "\x1b]777;notify;evil ]0,pwned ;line one line two 2J \x07\x07"
            .as_bytes()
            .to_vec()
    );
}

#[test]
fn a_semicolon_in_the_title_does_not_shift_the_body() {
    assert_eq!(
        terminal_attention("a;b", "c;d"),
        b"\x1b]777;notify;a,b;c;d\x07\x07".to_vec()
    );
}
