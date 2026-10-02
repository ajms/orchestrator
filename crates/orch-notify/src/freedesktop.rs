use std::collections::HashMap;
use std::sync::mpsc::{self, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use orch_core::SessionId;
use zbus::blocking::Connection;
use zbus::blocking::connection::Builder;
use zbus::zvariant::Value;

use crate::desktop::{
    ActionKey, Bus, BusError, ClickRoutes, DesktopSink, NotificationId, NotifyCall,
};
use crate::policy::DesktopAction;
use crate::sink::NotificationSink;

pub const QUEUE_CAPACITY: usize = 64;
pub const CALL_TIMEOUT: Duration = Duration::from_secs(5);

#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;

    fn close_notification(&self, id: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn action_invoked(&self, id: u32, action_key: String) -> zbus::Result<()>;
}

#[derive(Clone)]
pub struct FreedesktopBus {
    connection: Connection,
    proxy: NotificationsProxyBlocking<'static>,
}

impl FreedesktopBus {
    pub fn connect() -> Result<Self, BusError> {
        Self::open(Builder::session().map_err(bus_error)?)
    }

    fn open(builder: Builder<'_>) -> Result<Self, BusError> {
        let connection = builder
            .method_timeout(CALL_TIMEOUT)
            .build()
            .map_err(bus_error)?;
        let proxy = NotificationsProxyBlocking::new(&connection).map_err(bus_error)?;
        Ok(Self { connection, proxy })
    }

    pub fn listen_for_clicks(
        &self,
        clicks: ClickRoutes,
        on_click: impl Fn(SessionId) + Send + 'static,
    ) -> Result<JoinHandle<()>, BusError> {
        let signals = self.proxy.receive_action_invoked().map_err(bus_error)?;
        thread::Builder::new()
            .name("orch-notify-clicks".into())
            .spawn(move || {
                for signal in signals {
                    let Ok(args) = signal.args() else { continue };
                    let action = ActionKey(args.action_key);
                    if let Some(session) = clicks.route(NotificationId(args.id), &action) {
                        on_click(session);
                    }
                }
            })
            .map_err(bus_error)
    }

    pub fn disconnect(self) {
        let _ = self.connection.close();
    }
}

impl Bus for FreedesktopBus {
    fn notify(&self, call: &NotifyCall) -> Result<NotificationId, BusError> {
        let actions: Vec<&str> = call
            .actions
            .iter()
            .flat_map(|(key, label)| [key.0.as_str(), label.as_str()])
            .collect();
        self.proxy
            .notify(
                &call.app_name,
                call.replaces.map_or(0, |id| id.0),
                &call.app_icon,
                &call.title,
                &call.body,
                &actions,
                HashMap::new(),
                call.expire.as_millis(),
            )
            .map(NotificationId)
            .map_err(bus_error)
    }

    fn close(&self, id: NotificationId) -> Result<(), BusError> {
        self.proxy.close_notification(id.0).map_err(bus_error)
    }
}

fn bus_error(err: impl std::fmt::Display) -> BusError {
    BusError(err.to_string())
}

pub struct FreedesktopSink {
    actions: mpsc::SyncSender<DesktopAction>,
    worker_gone: bool,
}

impl FreedesktopSink {
    pub fn spawn(on_click: impl Fn(SessionId) + Send + 'static) -> Self {
        let (actions, pending) = mpsc::sync_channel(QUEUE_CAPACITY);
        let spawned = thread::Builder::new()
            .name("orch-notify".into())
            .spawn(move || match FreedesktopBus::connect() {
                Ok(bus) => deliver(bus, on_click, pending),
                Err(err) => disabled(&err),
            });
        if let Err(err) = spawned {
            disabled(&err);
        }
        Self {
            actions,
            worker_gone: false,
        }
    }
}

fn disabled(err: &dyn std::fmt::Display) {
    eprintln!("orch-notify: desktop notifications disabled: {err}");
}

fn deliver(
    bus: FreedesktopBus,
    on_click: impl Fn(SessionId) + Send + 'static,
    pending: mpsc::Receiver<DesktopAction>,
) {
    let mut sink = DesktopSink::new(bus.clone());
    let listener = bus
        .listen_for_clicks(sink.clicks(), on_click)
        .inspect_err(|err| eprintln!("orch-notify: notification clicks unavailable: {err}"))
        .ok();
    for action in pending {
        sink.apply(&action);
    }
    drop(sink);
    bus.disconnect();
    if let Some(listener) = listener {
        let _ = listener.join();
    }
}

impl NotificationSink for FreedesktopSink {
    fn apply(&mut self, action: &DesktopAction) {
        match self.actions.try_send(action.clone()) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                eprintln!("orch-notify: desktop notification dropped: queue full");
            }
            Err(TrySendError::Disconnected(_)) if !self.worker_gone => {
                self.worker_gone = true;
                eprintln!("orch-notify: desktop notifications stopped: worker has exited");
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }
}
