mod common;

use common::{close_merged, close_session, notification, session, show_merged, show_session};
use orch_notify::{NotificationKey, NotificationSink, RecordingSink};

#[test]
fn records_every_action_in_order() {
    let mut sink = RecordingSink::new();
    let actions = [show_session("a", "a", ""), close_session("a")];

    for action in &actions {
        sink.apply(action);
    }

    assert_eq!(sink.actions(), actions);
}

#[test]
fn shows_replace_in_place_and_closes_remove() {
    let mut sink = RecordingSink::new();

    sink.apply(&show_session("a", "a", "first"));
    sink.apply(&show_merged("2 Sessions need you", "", "b"));
    sink.apply(&show_session("a", "a", "second"));
    sink.apply(&close_merged());

    assert_eq!(
        sink.visible(),
        vec![(
            &NotificationKey::Session(session("a")),
            &notification("a", "second", "a")
        )]
    );
}

#[test]
fn clicking_a_visible_notification_yields_the_session_to_focus() {
    let mut sink = RecordingSink::new();
    sink.apply(&show_merged("2 Sessions need you", "", "b"));

    assert_eq!(sink.click(&NotificationKey::Merged), Some(session("b")));
    assert_eq!(sink.click(&NotificationKey::Session(session("b"))), None);
}
