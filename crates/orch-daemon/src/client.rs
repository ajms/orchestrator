use std::sync::Arc;

use orch_holder::{read_frame_async, write_frame_async};
use orch_protocol::{FromDaemon, PROTOCOL_VERSION, Reply, Request, RequestError, ToDaemon};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::task::JoinSet;

use crate::outbox::Outbox;
use crate::pane;
use crate::state::{ClientId, Daemon};

pub(crate) async fn serve(daemon: Arc<Daemon>, stream: UnixStream) {
    let (mut reader, mut writer) = stream.into_split();
    let Ok(Some(ToDaemon::Hello { version, pane })) = read_frame_async(&mut reader).await else {
        return;
    };
    if version != PROTOCOL_VERSION {
        let mismatch = FromDaemon::VersionMismatch {
            daemon_version: PROTOCOL_VERSION,
            message: format!(
                "the running Daemon speaks protocol {PROTOCOL_VERSION} but this Client speaks {version}; restart the Daemon to upgrade it"
            ),
        };
        let _ = write_frame_async(&mut writer, &mismatch).await;
        return;
    }
    let welcome = FromDaemon::Welcome {
        version: PROTOCOL_VERSION,
    };
    if write_frame_async(&mut writer, &welcome).await.is_err() {
        return;
    }
    if let Some(open) = pane {
        pane::serve(daemon, reader, writer, open).await;
        return;
    }
    let outbox = Arc::new(Outbox::default());
    let client = daemon.lock().add_client(outbox.clone());
    let mut writing = tokio::spawn(write_loop(writer, outbox.clone()));
    let mut requests = JoinSet::new();
    loop {
        let message = tokio::select! {
            message = read_frame_async::<ToDaemon>(&mut reader) => message,
            _ = &mut writing => break,
        };
        let Ok(Some(message)) = message else { break };
        if let ToDaemon::Request { id, request } = message {
            let daemon = daemon.clone();
            let outbox = outbox.clone();
            requests.spawn(async move {
                let result = handle(&daemon, client, request).await;
                daemon.store.flush().await;
                outbox.send(FromDaemon::Response { id, result });
            });
        }
        while requests.try_join_next().is_some() {}
    }
    requests.abort_all();
    daemon.lock().remove_client(client);
    writing.abort();
}

async fn write_loop(mut writer: OwnedWriteHalf, outbox: Arc<Outbox>) {
    while let Some(batch) = outbox.next_batch().await {
        for message in batch {
            if write_frame_async(&mut writer, &message).await.is_err() {
                return;
            }
        }
    }
}

async fn handle(
    daemon: &Arc<Daemon>,
    client: ClientId,
    request: Request,
) -> Result<Reply, RequestError> {
    match request {
        Request::View { session, focused } => {
            daemon.lock().set_view(client, session, focused);
            Ok(Reply::Done)
        }
        Request::CreateSession(create) => {
            let daemon = daemon.clone();
            detached(async move { daemon.create_session(create).await }).await
        }
        Request::ApproveTrust { repo, hash } => daemon.approve_trust(repo, hash).await,
        Request::RetrySetup { session } => daemon.retry_setup(&session),
        Request::StartAnyway { session } => daemon.start_anyway(&session),
        Request::Resume { session } => {
            let daemon = daemon.clone();
            detached(async move { daemon.resume(&session).await }).await
        }
        Request::AnswerGuard {
            session,
            guard,
            choice,
        } => daemon.answer_guard(&session, guard, choice),
        Request::SetGuards { session, enabled } => daemon.set_guards(&session, enabled),
        Request::Draft { session, mode } => daemon.draft(&session, mode).await,
        Request::Land { session, landing } => {
            let daemon = daemon.clone();
            detached(async move { daemon.land(&session, landing).await }).await
        }
        Request::RefreshPr { session } => {
            let daemon = daemon.clone();
            detached(async move { daemon.refresh_pr(&session).await }).await
        }
        Request::AbandonPr { session } => daemon.abandon_pr(&session),
        Request::DiscardPreview { session } => daemon.discard_preview(&session).await,
        Request::Discard { session } => {
            let daemon = daemon.clone();
            detached(async move { daemon.discard(&session).await }).await
        }
        Request::SetPreset { session, preset } => {
            let daemon = daemon.clone();
            detached(async move { daemon.set_preset(&session, preset).await }).await
        }
        Request::SetMuted { session, muted } => daemon.set_muted(&session, muted),
        Request::Reconcile => {
            let daemon = daemon.clone();
            detached(async move { daemon.reconcile_now().await }).await
        }
        Request::LeftoverPreview { repo, leftover } => {
            daemon.leftover_preview(repo, leftover).await
        }
        Request::Fix { fix } => {
            let daemon = daemon.clone();
            detached(async move { daemon.fix(fix).await }).await
        }
    }
}

async fn detached(
    work: impl Future<Output = Result<Reply, RequestError>> + Send + 'static,
) -> Result<Reply, RequestError> {
    tokio::spawn(work).await.unwrap_or_else(|err| {
        Err(RequestError::Refused {
            message: err.to_string(),
        })
    })
}
