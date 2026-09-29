mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use common::{close_session, session, show_merged, show_session};
use orch_notify::{
    ActionKey, Bus, BusError, DesktopSink, ExpireTimeout, NotificationId, NotificationSink,
    NotifyCall,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Notify(NotifyCall),
    Close(NotificationId),
}

#[derive(Clone, Default)]
struct FakeBus {
    calls: Rc<RefCell<Vec<Call>>>,
    next_id: Rc<RefCell<u32>>,
    down: Rc<RefCell<bool>>,
}

impl FakeBus {
    fn calls(&self) -> Vec<Call> {
        self.calls.borrow().clone()
    }

    fn last_notify(&self) -> NotifyCall {
        self.calls()
            .into_iter()
            .rev()
            .find_map(|call| match call {
                Call::Notify(notify) => Some(notify),
                Call::Close(_) => None,
            })
            .unwrap()
    }
}

impl Bus for FakeBus {
    fn notify(&self, call: &NotifyCall) -> Result<NotificationId, BusError> {
        if *self.down.borrow() {
            return Err(BusError("no session bus".into()));
        }
        self.calls.borrow_mut().push(Call::Notify(call.clone()));
        let mut next = self.next_id.borrow_mut();
        *next += 1;
        Ok(NotificationId(*next + 100))
    }

    fn close(&self, id: NotificationId) -> Result<(), BusError> {
        self.calls.borrow_mut().push(Call::Close(id));
        Ok(())
    }
}

#[test]
fn a_new_notification_is_sent_with_a_default_action_and_escaped_body() {
    let bus = FakeBus::default();
    let mut sink = DesktopSink::new(bus.clone());

    sink.apply(&show_session("a", "fix <login>", "Needs input · a&b <i>"));

    assert_eq!(
        bus.calls(),
        vec![Call::Notify(NotifyCall {
            app_name: "orch".into(),
            replaces: None,
            app_icon: String::new(),
            title: "fix <login>".into(),
            body: "Needs input · a&amp;b &lt;i&gt;".into(),
            actions: vec![(ActionKey::default_action(), "Open".into())],
            expire: ExpireTimeout::ServerDefault,
        })]
    );
}

#[test]
fn showing_the_same_key_again_replaces_the_notification_in_place() {
    let bus = FakeBus::default();
    let mut sink = DesktopSink::new(bus.clone());

    sink.apply(&show_session("a", "a", "first"));
    sink.apply(&show_session("b", "b", "other"));
    sink.apply(&show_session("a", "a", "second"));

    assert_eq!(bus.last_notify().replaces, Some(NotificationId(101)));
    assert_eq!(bus.last_notify().body, "second");
}

#[test]
fn closing_uses_the_bus_id_and_forgets_it() {
    let bus = FakeBus::default();
    let mut sink = DesktopSink::new(bus.clone());

    sink.apply(&show_session("a", "a", "first"));
    sink.apply(&close_session("a"));
    sink.apply(&close_session("a"));
    sink.apply(&show_session("a", "a", "again"));

    assert_eq!(bus.calls()[1], Call::Close(NotificationId(101)));
    assert_eq!(bus.calls().len(), 3);
    assert_eq!(bus.last_notify().replaces, None);
}

#[test]
fn the_default_action_routes_to_the_session_to_focus() {
    let bus = FakeBus::default();
    let mut sink = DesktopSink::new(bus.clone());
    let clicks = sink.clicks();

    sink.apply(&show_session("a", "a", "first"));
    sink.apply(&show_merged("2 Sessions need you", "", "b"));

    assert_eq!(
        clicks.route(NotificationId(101), &ActionKey("default".into())),
        Some(session("a"))
    );
    assert_eq!(
        clicks.route(NotificationId(102), &ActionKey("default".into())),
        Some(session("b"))
    );
    assert_eq!(
        clicks.route(NotificationId(101), &ActionKey("something-else".into())),
        None
    );
    assert_eq!(
        clicks.route(NotificationId(999), &ActionKey("default".into())),
        None
    );

    sink.apply(&close_session("a"));
    assert_eq!(
        clicks.route(NotificationId(101), &ActionKey("default".into())),
        None
    );
}

#[test]
fn a_missing_bus_is_tolerated() {
    let bus = FakeBus::default();
    let mut sink = DesktopSink::new(bus.clone());
    *bus.down.borrow_mut() = true;

    sink.apply(&show_session("a", "a", "lost"));
    sink.apply(&close_session("a"));
    *bus.down.borrow_mut() = false;
    sink.apply(&show_session("a", "a", "back"));

    assert_eq!(
        bus.calls()
            .into_iter()
            .filter(|c| matches!(c, Call::Close(_)))
            .count(),
        0
    );
    assert_eq!(bus.last_notify().replaces, None);
}

#[test]
fn expire_timeouts_map_to_the_wire_values() {
    assert_eq!(ExpireTimeout::ServerDefault.as_millis(), -1);
    assert_eq!(ExpireTimeout::Never.as_millis(), 0);
    assert_eq!(
        ExpireTimeout::After(Duration::from_secs(2)).as_millis(),
        2000
    );
}
