use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::net::UnixStream;
use tokio::sync::Notify;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender, error::TrySendError};

use crate::frame::{read_frame_async, write_frame_async};
use crate::holder::{Shared, handle};
use crate::{FromHolder, ScreenCopy, ScreenSnapshot, ToHolder};

const SCREEN_QUEUE: usize = 64;

pub(crate) type ConnId = u64;

#[derive(Clone)]
pub(crate) struct Outbox {
    control: UnboundedSender<FromHolder>,
    close: Arc<Notify>,
}

impl Outbox {
    pub(crate) fn send(&self, message: FromHolder) {
        let _ = self.control.send(message);
    }

    pub(crate) fn close_with(&self, message: FromHolder) {
        self.send(message);
        self.close.notify_one();
    }
}

pub(crate) enum ScreenItem {
    Snapshot(Box<ScreenCopy>),
    Frame(FromHolder),
}

#[derive(Clone)]
pub(crate) struct ScreenFeed {
    items: mpsc::Sender<ScreenItem>,
    lagged: Arc<AtomicBool>,
}

impl ScreenFeed {
    pub(crate) fn start(&self, copy: ScreenCopy) {
        self.lagged.store(false, Ordering::Release);
        let _ = self.items.try_send(ScreenItem::Snapshot(Box::new(copy)));
    }

    pub(crate) fn push(&self, message: FromHolder) -> bool {
        if self.lagged.load(Ordering::Acquire) {
            return true;
        }
        match self.items.try_send(ScreenItem::Frame(message)) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                self.lagged.store(true, Ordering::Release);
                true
            }
            Err(TrySendError::Closed(_)) => false,
        }
    }
}

pub(crate) async fn serve_connection(shared: Arc<Shared>, stream: UnixStream, conn: ConnId) {
    let (mut reader, writer) = stream.into_split();
    let (control, control_rx) = mpsc::unbounded_channel();
    let (items, items_rx) = mpsc::channel(SCREEN_QUEUE);
    let close = Arc::new(Notify::new());
    let outbox = Outbox {
        control,
        close: close.clone(),
    };
    let feed = ScreenFeed {
        items,
        lagged: Arc::new(AtomicBool::new(false)),
    };
    let writing = tokio::spawn(write_loop(
        shared.clone(),
        writer,
        control_rx,
        items_rx,
        feed.lagged.clone(),
    ));
    loop {
        let message = tokio::select! {
            message = read_frame_async::<ToHolder>(&mut reader) => message,
            () = close.notified() => break,
        };
        let Ok(Some(message)) = message else { break };
        if !handle(&shared, conn, &outbox, &feed, message).await {
            break;
        }
    }
    shared.disconnect(conn);
    drop((outbox, feed));
    let _ = writing.await;
}

async fn write_loop(
    shared: Arc<Shared>,
    mut writer: tokio::net::unix::OwnedWriteHalf,
    mut control: UnboundedReceiver<FromHolder>,
    mut screen: mpsc::Receiver<ScreenItem>,
    lagged: Arc<AtomicBool>,
) {
    loop {
        let message = tokio::select! {
            biased;
            message = control.recv() => match message {
                Some(message) => message,
                None => break,
            },
            Some(item) = screen.recv() => {
                if lagged.load(Ordering::Acquire) {
                    while screen.try_recv().is_ok() {}
                    let copy = shared.resync(&lagged);
                    FromHolder::Screen(capture(copy).await)
                } else {
                    match item {
                        ScreenItem::Snapshot(copy) => FromHolder::Screen(capture(*copy).await),
                        ScreenItem::Frame(message) => message,
                    }
                }
            }
        };
        if write_frame_async(&mut writer, &message).await.is_err() {
            break;
        }
    }
}

pub(crate) async fn capture(copy: ScreenCopy) -> ScreenSnapshot {
    tokio::task::spawn_blocking(move || copy.capture())
        .await
        .expect("screen capture panicked")
}
