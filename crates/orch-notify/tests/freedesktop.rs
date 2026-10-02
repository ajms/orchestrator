mod common;

use std::sync::mpsc;
use std::time::Duration;

use common::{close_session, show_session};
use orch_notify::{
    ActionKey, Bus, ClickRoutes, ExpireTimeout, FreedesktopBus, FreedesktopSink, NotificationSink,
    NotifyCall,
};

#[test]
#[ignore = "needs a session bus"]
fn the_click_listener_stops_when_the_bus_disconnects() {
    let bus = FreedesktopBus::connect().expect("session bus");
    let listener = bus
        .listen_for_clicks(ClickRoutes::default(), |_session| {})
        .expect("subscribe to ActionInvoked");
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = listener.join();
        let _ = done.send(());
    });

    bus.disconnect();

    finished
        .recv_timeout(Duration::from_secs(5))
        .expect("listener thread exits");
}

#[test]
#[ignore = "needs a session bus with a notification server"]
fn the_real_notification_server_accepts_replace_and_close() {
    let bus = FreedesktopBus::connect().expect("session bus");
    let call = NotifyCall {
        app_name: "orch".into(),
        replaces: None,
        app_icon: String::new(),
        title: "orch test".into(),
        body: "first".into(),
        actions: vec![(ActionKey::default_action(), "Open".into())],
        expire: ExpireTimeout::After(Duration::from_secs(2)),
    };
    let id = bus.notify(&call).unwrap();
    let replaced = bus
        .notify(&NotifyCall {
            replaces: Some(id),
            body: "second".into(),
            ..call
        })
        .unwrap();
    assert_eq!(replaced, id);
    bus.close(id).unwrap();
}

#[test]
#[ignore = "needs a session bus with a notification server"]
fn the_threaded_sink_delivers_without_blocking_the_caller() {
    let mut sink = FreedesktopSink::spawn(|_session| {});
    sink.apply(&show_session("orch-test", "orch test", "threaded"));
    std::thread::sleep(Duration::from_millis(500));
    sink.apply(&close_session("orch-test"));
}
