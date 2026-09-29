mod common;

use common::Fixture;
use orch_config::{Channel, ConfigError, ConfigProblem};
use orch_core::Attention;

#[test]
fn every_trigger_notifies_on_every_channel_by_default() {
    let fx = Fixture::new();
    let global = fx.loader.global().unwrap();
    for attention in [
        Attention::NeedsInput,
        Attention::TurnEnded,
        Attention::PrClosed,
    ] {
        assert!(global.notifications.enabled(Channel::Desktop, attention));
        assert!(global.notifications.enabled(Channel::Bell, attention));
    }
}

#[test]
fn triggers_are_configured_per_channel_globally() {
    let fx = Fixture::new();
    fx.global("[notifications.desktop]\nturn_ended = false\n");
    let global = fx.loader.global().unwrap();
    assert!(
        !global
            .notifications
            .enabled(Channel::Desktop, Attention::TurnEnded)
    );
    assert!(
        global
            .notifications
            .enabled(Channel::Bell, Attention::TurnEnded)
    );
    assert!(
        global
            .notifications
            .enabled(Channel::Desktop, Attention::NeedsInput)
    );
}

#[test]
fn repo_overrides_individual_triggers_on_top_of_the_global_ones() {
    let fx = Fixture::new();
    fx.global(&format!(
        "[notifications.desktop]\nturn_ended = false\nchecks_failing = false\n\n[repos.{}.notifications.desktop]\nchecks_failing = true\n",
        fx.repo_key()
    ));
    fx.repo_file("[notifications.desktop]\nturn_ended = true\nchecks_failing = false\n[notifications.bell]\nneeds_input = false\n");

    let config = fx.untrusted();
    let notifications = config.notifications();
    assert!(notifications.enabled(Channel::Desktop, Attention::TurnEnded));
    assert!(notifications.enabled(Channel::Desktop, Attention::ChecksFailing));
    assert!(!notifications.enabled(Channel::Bell, Attention::NeedsInput));
    assert!(notifications.enabled(Channel::Desktop, Attention::NeedsInput));
}

#[test]
fn unknown_trigger_names_are_rejected() {
    let fx = Fixture::new();
    fx.global("[notifications.desktop]\nsneezed = false\n");
    assert!(fx.loader.global().is_err());
}

#[test]
fn global_triggers_live_only_at_the_top_level() {
    let fx = Fixture::new();
    fx.global("[defaults.notifications.desktop]\nturn_ended = false\n");
    assert!(matches!(
        fx.loader.global(),
        Err(ConfigError::Invalid {
            problem: ConfigProblem::Misplaced {
                key: "notifications"
            },
            ..
        })
    ));
}
