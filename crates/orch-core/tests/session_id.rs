use orch_core::SessionId;

#[test]
fn session_ids_are_path_and_shell_safe_names() {
    for valid in ["s1", "fix-login_2", "a.b", "0f9c6a52-6a3e-4d3b"] {
        assert_eq!(SessionId::parse(valid).unwrap().as_str(), valid);
    }
}

#[test]
fn session_ids_that_could_escape_a_directory_are_rejected() {
    for invalid in [
        "",
        ".",
        "..",
        ".hidden",
        "a/b",
        "a b",
        "semi;colon",
        "quote'",
    ] {
        assert!(SessionId::parse(invalid).is_err(), "{invalid:?} accepted");
    }
}

#[test]
fn overlong_session_ids_are_rejected() {
    assert!(SessionId::parse(&"a".repeat(64)).is_ok());
    assert!(SessionId::parse(&"a".repeat(65)).is_err());
}
