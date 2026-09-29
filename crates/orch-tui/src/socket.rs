use std::path::{Path, PathBuf};

use orch_core::SessionId;
use orch_holder::{read_frame_async, write_frame_async};
use orch_protocol::{ConnectError, FromDaemon, OpenPane, Request, Size, ToDaemon, open_connection};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;

use crate::event::{Event, PaneId};
use crate::link::{DaemonLink, RequestId};

const OFFLINE: &str = "not connected to the Daemon; restart orch to reconnect";

pub struct SocketLink {
    socket: PathBuf,
    events: UnboundedSender<Event>,
    control: Option<Relay>,
    pane: Option<Relay>,
}

struct Relay {
    outbox: UnboundedSender<ToDaemon>,
    tasks: Vec<JoinHandle<()>>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl SocketLink {
    pub async fn connect(
        socket: &Path,
        events: UnboundedSender<Event>,
    ) -> Result<Self, ConnectError> {
        let (reader, writer) = open_connection(socket, None).await?;
        let (outbox, pending) = unbounded_channel();
        let relay = events.clone();
        let reading = tokio::spawn(async move {
            let reason = relay_loop(reader, Event::Daemon, &relay).await;
            let _ = relay.send(Event::Disconnected { reason });
        });
        let writing = tokio::spawn(write_loop(writer, pending));
        Ok(Self {
            socket: socket.to_path_buf(),
            events,
            control: Some(Relay {
                outbox,
                tasks: vec![reading, writing],
            }),
            pane: None,
        })
    }

    pub fn offline(socket: &Path, events: UnboundedSender<Event>) -> Self {
        Self {
            socket: socket.to_path_buf(),
            events,
            control: None,
            pane: None,
        }
    }

    fn send_pane(&mut self, message: ToDaemon) {
        if let Some(pane) = &self.pane {
            let _ = pane.outbox.send(message);
        }
    }
}

impl DaemonLink for SocketLink {
    fn request(&mut self, id: RequestId, request: Request) {
        let message = ToDaemon::Request { id: id.0, request };
        let sent = self
            .control
            .as_ref()
            .is_some_and(|control| control.outbox.send(message).is_ok());
        if !sent {
            let _ = self.events.send(Event::Notice(OFFLINE.into()));
        }
    }

    fn open_pane(&mut self, pane: PaneId, session: &SessionId, size: Size) {
        let (outbox, pending) = unbounded_channel();
        let open = OpenPane {
            session: session.clone(),
            size,
        };
        let task = tokio::spawn(pane_loop(
            self.socket.clone(),
            pane,
            open,
            pending,
            self.events.clone(),
        ));
        self.pane = Some(Relay {
            outbox,
            tasks: vec![task],
        });
    }

    fn close_pane(&mut self) {
        self.pane = None;
    }

    fn input(&mut self, bytes: Vec<u8>) {
        self.send_pane(ToDaemon::Input { bytes });
    }

    fn paste(&mut self, text: String) {
        self.send_pane(ToDaemon::Paste { text });
    }

    fn resize_pane(&mut self, size: Size) {
        self.send_pane(ToDaemon::Resize(size));
    }
}

async fn write_loop(mut writer: OwnedWriteHalf, mut outbox: UnboundedReceiver<ToDaemon>) {
    while let Some(message) = outbox.recv().await {
        if write_frame_async(&mut writer, &message).await.is_err() {
            break;
        }
    }
}

async fn relay_loop(
    mut reader: OwnedReadHalf,
    wrap: impl Fn(FromDaemon) -> Event,
    events: &UnboundedSender<Event>,
) -> String {
    loop {
        match read_frame_async::<FromDaemon>(&mut reader).await {
            Ok(Some(message)) => {
                if events.send(wrap(message)).is_err() {
                    return "the Client stopped".into();
                }
            }
            Ok(None) => return "the Daemon closed the connection".into(),
            Err(err) => return err.to_string(),
        }
    }
}

async fn pane_loop(
    socket: PathBuf,
    pane: PaneId,
    open: OpenPane,
    outbox: UnboundedReceiver<ToDaemon>,
    events: UnboundedSender<Event>,
) {
    let wrap = |message| Event::Pane { pane, message };
    let (reader, writer) = match open_connection(&socket, Some(open)).await {
        Ok(halves) => halves,
        Err(err) => {
            let reason = err.to_string();
            let _ = events.send(wrap(FromDaemon::PaneClosed { reason }));
            return;
        }
    };
    let writing = tokio::spawn(write_loop(writer, outbox));
    relay_loop(reader, wrap, &events).await;
    writing.abort();
}
