use orch_agent::{SandboxFailure, sandbox_refusal};

const UID_MAP: &str = "bwrap: setting up uid map: Permission denied";

fn host(settings: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let settings: Vec<(String, String)> = settings
        .iter()
        .map(|(key, value)| (key.to_string(), format!("{value}\n")))
        .collect();
    move |key| {
        settings
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.clone())
    }
}

fn failed() -> SandboxFailure {
    SandboxFailure::Failed(UID_MAP.into())
}

#[test]
fn a_missing_bwrap_says_to_install_bubblewrap() {
    let message = sandbox_refusal(&SandboxFailure::Missing, host(&[]));
    assert!(message.contains("bwrap is not installed"), "{message}");
    assert!(message.contains("install bubblewrap"), "{message}");
}

#[test]
fn disabled_unprivileged_user_namespaces_are_named() {
    let message = sandbox_refusal(
        &failed(),
        host(&[("kernel.unprivileged_userns_clone", "0")]),
    );
    assert!(
        message.contains("kernel.unprivileged_userns_clone is 0"),
        "{message}"
    );
}

#[test]
fn no_room_for_user_namespaces_is_named() {
    let message = sandbox_refusal(&failed(), host(&[("user.max_user_namespaces", "0")]));
    assert!(
        message.contains("user.max_user_namespaces is 0"),
        "{message}"
    );
}

#[test]
fn ubuntus_apparmor_restriction_asks_for_a_bwrap_profile() {
    let message = sandbox_refusal(
        &failed(),
        host(&[
            ("kernel.unprivileged_userns_clone", "1"),
            ("user.max_user_namespaces", "63000"),
            ("kernel.apparmor_restrict_unprivileged_userns", "1"),
        ]),
    );
    assert!(
        message.contains("kernel.apparmor_restrict_unprivileged_userns is 1"),
        "{message}"
    );
    assert!(message.contains("AppArmor profile for bwrap"), "{message}");
}

#[test]
fn an_unexplained_failure_shows_what_bwrap_said() {
    let message = sandbox_refusal(&failed(), host(&[("user.max_user_namespaces", "63000")]));
    assert!(message.contains(UID_MAP), "{message}");
    assert!(!message.contains(" is 0"), "{message}");
}

#[test]
fn every_refusal_offers_the_ways_out() {
    for message in [
        sandbox_refusal(&SandboxFailure::Missing, host(&[])),
        sandbox_refusal(&failed(), host(&[])),
    ] {
        assert!(message.starts_with("The Sandbox can't run"), "{message}");
        assert!(message.contains("[defaults] sandbox = false"), "{message}");
        assert!(
            message.contains(":preset to a Preset with sandbox = false"),
            "{message}"
        );
    }
}
