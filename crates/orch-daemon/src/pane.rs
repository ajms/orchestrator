use std::sync::Arc;
use std::time::Instant;

use orch_core::{Observation, SessionId};
use orch_holder::{FromHolder, HolderClient, Size, ToHolder, read_frame_async, write_frame_async};
use orch_protocol::{FromDaemon, OpenPane, ToDaemon};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;

use crate::state::{Daemon, PaneId};

enum Activity {
    Opened(Size),
    Resized(Size),
    Typed,
}

pub(crate) async fn serve(
    daemon: Arc<Daemon>,
    mut reader: OwnedReadHalf,
    mut writer: OwnedWriteHalf,
    open: OpenPane,
) {
    let OpenPane { session, size } = open;
    let live_holder = daemon
        .lock()
        .sessions
        .get(&session)
        .is_some_and(|live| live.has_holder());
    let holder = match live_holder {
        true => HolderClient::connect(&daemon.holder_socket(&session))
            .await
            .ok(),
        false => None,
    };
    let Some(mut holder) = holder else {
        let closed = FromDaemon::PaneClosed {
            reason: "the Session has no live Holder".into(),
        };
        let _ = write_frame_async(&mut writer, &closed).await;
        return;
    };
    if holder.send(&ToHolder::Subscribe).await.is_err() {
        return;
    }
    let pane = daemon.lock().next_id();
    daemon.pane_activity(&session, pane, Activity::Opened(size));
    let (mut from_holder, mut to_holder) = holder.into_split();

    let (notices, mut notice_queue) = mpsc::unbounded_channel::<FromDaemon>();
    let mut relay_out = tokio::spawn(async move {
        loop {
            let received = tokio::select! {
                received = from_holder.recv() => received,
                Some(notice) = notice_queue.recv() => {
                    if write_frame_async(&mut writer, &notice).await.is_err() {
                        return;
                    }
                    continue;
                }
            };
            let message = match received {
                Ok(Some(FromHolder::Screen(snapshot))) => FromDaemon::Screen(snapshot),
                Ok(Some(FromHolder::Output { bytes })) => FromDaemon::Output { bytes },
                Ok(Some(FromHolder::Resized(size))) => FromDaemon::Resized(size),
                Ok(Some(_)) => continue,
                Ok(None) | Err(_) => {
                    let closed = FromDaemon::PaneClosed {
                        reason: "the Holder went away".into(),
                    };
                    let _ = write_frame_async(&mut writer, &closed).await;
                    return;
                }
            };
            if write_frame_async(&mut writer, &message).await.is_err() {
                return;
            }
        }
    });
    let relay_daemon = daemon.clone();
    let relay_session = session.clone();
    let mut relay_in = tokio::spawn(async move {
        while let Ok(Some(message)) = read_frame_async::<ToDaemon>(&mut reader).await {
            let (forward, activity) = match message {
                ToDaemon::Input { bytes } => (Some(ToHolder::Input { bytes }), Activity::Typed),
                ToDaemon::Paste { text } => (Some(ToHolder::Paste { text }), Activity::Typed),
                ToDaemon::Resize(size) => (None, Activity::Resized(size)),
                _ => continue,
            };
            if forward.is_some() && relay_daemon.is_being_ended(&relay_session) {
                let _ = notices.send(FromDaemon::InputDropped {
                    reason: "the Session is being Landed or Discarded; input is ignored until that finishes".into(),
                });
                continue;
            }
            if let Some(forward) = forward
                && write_frame_async(&mut to_holder, &forward).await.is_err()
            {
                return;
            }
            relay_daemon.pane_activity(&relay_session, pane, activity);
        }
    });
    tokio::select! {
        _ = &mut relay_out => relay_in.abort(),
        _ = &mut relay_in => relay_out.abort(),
    }
    daemon.close_pane(&session, pane);
}

impl Daemon {
    fn is_being_ended(&self, id: &SessionId) -> bool {
        self.lock()
            .sessions
            .get(id)
            .is_some_and(|live| live.exclusive)
    }

    fn pane_activity(&self, id: &SessionId, pane: PaneId, activity: Activity) {
        let mut state = self.lock();
        let Some(live) = state.sessions.get_mut(id) else {
            return;
        };
        let size = match activity {
            Activity::Opened(size) | Activity::Resized(size) => Some(size),
            Activity::Typed => None,
        };
        live.panes.touch(pane, size);
        live.apply_pane_size();
        if matches!(activity, Activity::Typed) {
            let before = live.status.agent_state();
            live.status.observe(Observation::UserInput, Instant::now());
            if live.status.agent_state() != before {
                state.changed(id);
            }
        }
    }

    fn close_pane(&self, id: &SessionId, pane: PaneId) {
        let mut state = self.lock();
        if let Some(live) = state.sessions.get_mut(id) {
            live.panes.close(pane);
            live.apply_pane_size();
        }
    }
}
