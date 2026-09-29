use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nix::sys::signal::kill;
use nix::unistd::Pid;
use orch_agent::{GuardAnswer, GuardContext, GuardDecision, evaluate_guard};
use orch_core::{
    AgentEvent, AgentState, ConversationId, Effect, Observation, PhaseEvent, SessionId,
};
use orch_holder::{
    AgentExit, AgentStatus, FromHolder, HolderClient, HolderEvent, HolderReader, ToHolder,
    socket_path, write_frame_async,
};
use orch_protocol::{GuardChoice, Reply, RequestError};
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::mpsc::{self, Receiver};
use tokio::sync::watch;

use crate::lifecycle::PROMPT_FILE;
use crate::state::{Daemon, HolderLink, Live, PendingGuard};

const DENIED_BY_USER: &str = "The user denied this in the Orchestrator (Guard).";
const HOLDER_QUEUE: usize = 256;
const EXIT_POLL: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Attach {
    Fresh,
    Adopt,
    Reattach { last_seq: u64 },
}

impl Daemon {
    pub(crate) fn holder_socket(&self, id: &SessionId) -> PathBuf {
        socket_path(&self.config.runtime_dir, id)
    }

    pub(crate) async fn attach(self: &Arc<Self>, id: &SessionId, how: Attach) -> io::Result<()> {
        let mut client = HolderClient::connect(&self.holder_socket(id)).await?;
        let hello = client.attach().await?;
        let (reader, mut writer) = client.into_split();
        let (outbox, inbox) = mpsc::channel(HOLDER_QUEUE);
        let (closed, closed_watch) = watch::channel(false);
        let generation = {
            let mut state = self.lock();
            match state.sessions.get_mut(id) {
                Some(live) if !live.status.phase().is_terminal() => {
                    let link = HolderLink {
                        outbox,
                        pid: hello.holder_pid,
                        closed: closed_watch,
                    };
                    let (generation, _) = live.replace_holder(Some(link));
                    let running = hello.agent == AgentStatus::Running;
                    let now = Instant::now();
                    match (how, live.record.agent_state) {
                        (Attach::Reattach { .. }, _) => {}
                        (Attach::Adopt, Some(persisted)) => {
                            live.status.restore_agent(persisted, running)
                        }
                        _ => {
                            live.status.observe(Observation::Spawned, now);
                        }
                    }
                    if let AgentStatus::Exited(exit) = &hello.agent
                        && !matches!(
                            live.status.agent_state(),
                            Some(AgentState::Exited | AgentState::Errored)
                        )
                    {
                        live.status.observe(exit_observation(exit), now);
                    }
                    live.apply_pane_size();
                    live.deliver_prompt();
                    state.changed(id);
                    Some(generation)
                }
                _ => None,
            }
        };
        let Some(generation) = generation else {
            let _ = write_frame_async(&mut writer, &ToHolder::Shutdown).await;
            return Err(io::Error::other("the Session has ended"));
        };
        let last_seq = match how {
            Attach::Reattach { last_seq } => last_seq,
            _ => 0,
        };
        tokio::spawn(write_loop(writer, inbox));
        tokio::spawn(
            self.clone()
                .consume(id.clone(), reader, generation, last_seq, closed),
        );
        Ok(())
    }

    async fn consume(
        self: Arc<Self>,
        id: SessionId,
        mut reader: HolderReader,
        generation: u64,
        mut last_seq: u64,
        closed: watch::Sender<bool>,
    ) {
        while let Ok(Some(message)) = reader.recv().await {
            match message {
                FromHolder::Event { seq, event } if seq > last_seq => {
                    last_seq = seq;
                    let effects = self.on_holder_event(&id, generation, event, seq);
                    if effects.contains(&Effect::RecheckRebase) {
                        self.request_recheck(&id);
                    }
                }
                FromHolder::Superseded => break,
                _ => {}
            }
        }
        closed.send_replace(true);
        self.holder_lost(&id, generation, last_seq).await;
    }

    async fn holder_lost(self: &Arc<Self>, id: &SessionId, generation: u64, last_seq: u64) {
        let current = self
            .lock()
            .sessions
            .get(id)
            .is_some_and(|live| live.is_current(generation));
        if !current {
            return;
        }
        if self.reattach(id, last_seq).await.is_ok() {
            return;
        }
        let _ = self.update(id, |live| {
            if !live.is_current(generation) {
                return Ok(());
            }
            live.replace_holder(None);
            if live.status.phase().is_live() {
                live.transition(PhaseEvent::Suspended)?;
            }
            Ok(())
        });
    }

    fn reattach(
        self: &Arc<Self>,
        id: &SessionId,
        last_seq: u64,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send>> {
        let daemon = self.clone();
        let id = id.clone();
        Box::pin(async move { daemon.attach(&id, Attach::Reattach { last_seq }).await })
    }

    pub(crate) async fn release_holder(&self, link: HolderLink, limit: Duration) {
        let _ = link.outbox.try_send(ToHolder::Shutdown);
        let mut closed = link.closed;
        let deadline = tokio::time::Instant::now() + limit;
        let _ = tokio::time::timeout_at(deadline, closed.wait_for(|closed| *closed)).await;
        let pid = Pid::from_raw(link.pid as i32);
        while kill(pid, None).is_ok() && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(EXIT_POLL).await;
        }
    }

    fn on_holder_event(
        &self,
        id: &SessionId,
        generation: u64,
        event: HolderEvent,
        seq: u64,
    ) -> Vec<Effect> {
        let mut state = self.lock();
        let Some(live) = state.sessions.get_mut(id) else {
            return Vec::new();
        };
        if !live.is_current(generation) {
            return Vec::new();
        }
        let now = Instant::now();
        let mut conversations = Vec::new();
        let mut usage = Vec::new();
        let effects = match event {
            HolderEvent::Spawned { .. } => live.status.observe(Observation::Spawned, now),
            HolderEvent::Exited(exit) => {
                live.prompts.clear();
                live.status.observe(exit_observation(&exit), now)
            }
            HolderEvent::Hook { payload, guard } => {
                let events = live.adapter.map_hook(&payload).unwrap_or_default();
                let effects = observe(live, &events, now, &mut conversations, &mut usage);
                if let Some(guard) = guard {
                    decide_guard(live, guard, &events, now);
                }
                effects
            }
            HolderEvent::Tap { payload } => {
                let events = live.adapter.map_tap(&payload).unwrap_or_default();
                observe(live, &events, now, &mut conversations, &mut usage)
            }
        };
        live.deliver_prompt();
        let first_conversation = live.record.conversations.is_empty() && !conversations.is_empty();
        for conversation in &conversations {
            if live.record.latest_conversation() != Some(conversation) {
                live.record.conversations.push(conversation.clone());
            }
        }
        let ack = live.holder_outbox();
        state.changed(id);
        let session = id.clone();
        let prompt = self.session_dir(id).join(PROMPT_FILE);
        self.store.write(move |store| {
            for conversation in conversations {
                if let Err(err) = store.record_conversation(&session, conversation) {
                    eprintln!("orch daemon: recording a Conversation: {err}");
                }
            }
            for sample in usage {
                let _ = store.record_usage(&session, &sample);
            }
            if first_conversation {
                let _ = std::fs::remove_file(prompt);
            }
            if let Some(ack) = ack {
                let _ = ack.try_send(ToHolder::Ack { through: seq });
            }
        });
        effects
    }

    pub(crate) fn answer_guard(
        &self,
        id: &SessionId,
        guard: u64,
        choice: GuardChoice,
    ) -> Result<Reply, RequestError> {
        self.update(id, |live| {
            let at = live
                .prompts
                .iter()
                .position(|pending| pending.id == guard)
                .ok_or("no such Guard prompt is pending")?;
            let pending = live.prompts.remove(at);
            let answer = match choice {
                GuardChoice::AllowOnce => GuardAnswer::Proceed,
                GuardChoice::AllowForSession => {
                    live.record.guard_allowances.push(pending.hit);
                    GuardAnswer::Proceed
                }
                GuardChoice::Deny => GuardAnswer::Deny {
                    reason: DENIED_BY_USER.into(),
                },
            };
            live.send_to_holder(ToHolder::GuardAnswer { id: guard, answer });
            Ok(())
        })?;
        Ok(Reply::Done)
    }

    pub(crate) fn set_guards(&self, id: &SessionId, enabled: bool) -> Result<Reply, RequestError> {
        self.update(id, |live| {
            live.record.guards_enabled = enabled;
            Ok(())
        })?;
        Ok(Reply::Done)
    }
}

fn observe(
    live: &mut Live,
    events: &[AgentEvent],
    now: Instant,
    conversations: &mut Vec<ConversationId>,
    usage: &mut Vec<orch_core::UsageSample>,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    for event in events {
        match event {
            AgentEvent::ConversationChanged { id } => conversations.push(id.clone()),
            AgentEvent::UsageSample(sample) => usage.push(sample.clone()),
            _ => {}
        }
        effects.extend(live.status.observe(Observation::Agent(event.clone()), now));
    }
    effects
}

fn decide_guard(live: &mut Live, guard: u64, events: &[AgentEvent], now: Instant) {
    let check = events.iter().find_map(|event| match event {
        AgentEvent::GuardCheck {
            tool,
            input_json,
            cwd,
        } => Some((tool, input_json, cwd)),
        _ => None,
    });
    let decision = check.map(|(tool, input_json, cwd)| {
        let context = GuardContext {
            worktree: &live.record.worktree,
            branch: &live.record.branch,
            base_branch: &live.record.base,
            enabled: live.record.guards_enabled,
            allowed: &live.record.guard_allowances,
        };
        let decision = evaluate_guard(tool, input_json, cwd.as_deref().map(Path::new), &context);
        (tool.clone(), decision)
    });
    match decision {
        Some((tool, GuardDecision::Ask(hit))) => {
            live.send_to_holder(ToHolder::GuardHeld { id: guard });
            live.prompts.push(PendingGuard {
                id: guard,
                tool,
                hit,
            });
            live.status.observe(Observation::GuardPrompted, now);
        }
        _ => {
            live.send_to_holder(ToHolder::GuardAnswer {
                id: guard,
                answer: GuardAnswer::Proceed,
            });
        }
    }
}

fn exit_observation(exit: &AgentExit) -> Observation {
    let code = match exit.signal {
        Some(_) => None,
        None => i32::try_from(exit.code).ok(),
    };
    Observation::Exited { code }
}

async fn write_loop(mut writer: OwnedWriteHalf, mut inbox: Receiver<ToHolder>) {
    while let Some(message) = inbox.recv().await {
        if write_frame_async(&mut writer, &message).await.is_err() {
            break;
        }
    }
}
