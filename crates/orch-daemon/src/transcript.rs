use std::sync::Arc;
use std::time::Duration;

use orch_agent::TranscriptReader;
use orch_core::{SessionId, SubagentId};
use orch_protocol::{FromDaemon, Reply, RequestError, SubagentTranscript};
use tokio::task::AbortHandle;

use crate::outbox::Outbox;
use crate::state::{ClientId, Daemon};

const TRANSCRIPT_POLL: Duration = Duration::from_millis(250);

pub(crate) struct Following(AbortHandle);

impl Drop for Following {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Daemon {
    pub(crate) fn subscribe_subagent(
        self: &Arc<Self>,
        client: ClientId,
        session: &SessionId,
        subagent: SubagentId,
    ) -> Result<Reply, RequestError> {
        let mut state = self.lock();
        let live = state
            .sessions
            .get(session)
            .ok_or(RequestError::UnknownSession)?;
        if !live
            .status
            .subagents()
            .iter()
            .any(|known| known.id == subagent)
        {
            return Err(refused("no such Subagent"));
        }
        let reader = live
            .transcripts
            .as_ref()
            .ok_or_else(|| refused("the Agent keeps no Subagent transcripts"))?
            .reader();
        let Some(outbox) = state.client_outbox(client) else {
            return Ok(Reply::Done);
        };
        let streaming =
            tokio::spawn(
                self.clone()
                    .stream_subagent(session.clone(), subagent, reader, outbox),
            );
        state.follow(client, Some(Following(streaming.abort_handle())));
        Ok(Reply::Done)
    }

    async fn stream_subagent(
        self: Arc<Self>,
        session: SessionId,
        subagent: SubagentId,
        mut reader: Box<dyn TranscriptReader>,
        outbox: Arc<Outbox>,
    ) {
        let mut tick = tokio::time::interval(TRANSCRIPT_POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut backlog = true;
        loop {
            tick.tick().await;
            let path = {
                let state = self.lock();
                let Some(live) = state.sessions.get(&session) else {
                    return;
                };
                live.transcripts
                    .as_ref()
                    .and_then(|transcripts| transcripts.locate(&subagent))
            };
            let read;
            (reader, read) = match path {
                Some(path) => {
                    let reading = tokio::task::spawn_blocking(move || {
                        let read = reader.read(&path);
                        (reader, read)
                    });
                    match reading.await {
                        Ok(done) => done,
                        Err(err) => {
                            eprintln!(
                                "orch daemon: reading the transcript of Subagent {}: {err}",
                                subagent.as_str()
                            );
                            return;
                        }
                    }
                }
                None => (reader, Default::default()),
            };
            let replace = backlog || read.reset;
            if replace || !read.entries.is_empty() {
                backlog = false;
                outbox.send(FromDaemon::SubagentTranscript(SubagentTranscript {
                    session: session.clone(),
                    subagent: subagent.as_str().into(),
                    entries: read.entries,
                    replace,
                }));
            }
        }
    }
}

fn refused(message: &str) -> RequestError {
    RequestError::Refused {
        message: message.into(),
    }
}
