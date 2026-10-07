use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nix::sys::signal::kill;
use nix::unistd::Pid;
use orch_agent::{
    Capabilities, GuardAnswer, GuardContext, GuardDecision, TitleWatch, evaluate_guard,
};
use orch_core::{
    AgentEvent, AgentState, ConversationId, Effect, GuardedAction, Observation, PhaseEvent,
    SessionId,
};
use orch_holder::{
    AgentExit, AgentStatus, FromHolder, HolderClient, HolderEvent, HolderReader, ToHolder,
    socket_path, write_frame_async,
};
use orch_protocol::{GuardChoice, Reply, RequestError};
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::watch;

use crate::lifecycle::PROMPT_FILE;
use crate::state::{Daemon, HolderLink, Live, PendingGuard};

const DENIED_BY_USER: &str = "The user denied this in the Orchestrator (Guard).";
const HOLDER_QUEUE: usize = 256;
const EXIT_POLL: Duration = Duration::from_millis(20);
const TITLE_POLL: Duration = Duration::from_secs(1);

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
                        port_block: hello.port_block,
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
        let titles = self.watch_titles(&id);
        while let Ok(Some(message)) = reader.recv().await {
            match message {
                FromHolder::Event { seq, event } if seq > last_seq => {
                    last_seq = seq;
                    if let (
                        Some(titles),
                        HolderEvent::Hook { payload, .. } | HolderEvent::Tap { payload },
                    ) = (&titles, &event)
                    {
                        let _ = titles.try_send(payload.clone());
                    }
                    let effects = self.on_holder_event(&id, generation, event, seq);
                    if effects.contains(&Effect::RecheckRebase) {
                        self.request_recheck(&id);
                    }
                }
                FromHolder::Clipboard { text } => self.lock().relay_copy(&id, text),
                FromHolder::Superseded => break,
                _ => {}
            }
        }
        drop(titles);
        closed.send_replace(true);
        self.holder_lost(&id, generation, last_seq).await;
    }

    fn watch_titles(self: &Arc<Self>, id: &SessionId) -> Option<Sender<String>> {
        let watch = self
            .lock()
            .sessions
            .get(id)
            .and_then(|live| live.title_watch())?;
        let (payloads, followed) = mpsc::channel(HOLDER_QUEUE);
        tokio::spawn(self.clone().follow_titles(id.clone(), watch, followed));
        Some(payloads)
    }

    async fn follow_titles(
        self: Arc<Self>,
        id: SessionId,
        mut watch: Box<dyn TitleWatch>,
        mut followed: Receiver<String>,
    ) {
        let mut tick = tokio::time::interval(TITLE_POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                payload = followed.recv() => match payload {
                    Some(payload) => watch.follow(&payload),
                    None => return,
                },
                _ = tick.tick() => {}
            }
            let Ok((back, events)) = tokio::task::spawn_blocking(move || {
                let events = watch.poll();
                (watch, events)
            })
            .await
            else {
                return;
            };
            watch = back;
            if !events.is_empty() {
                self.on_title_events(&id, &events);
            }
        }
    }

    fn on_title_events(&self, id: &SessionId, events: &[AgentEvent]) {
        let mut state = self.lock();
        let Some(live) = state.sessions.get_mut(id) else {
            return;
        };
        for event in events {
            if let AgentEvent::TitleChanged { title } = event {
                retitle(live, title);
            }
        }
        state.changed(id);
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
                if let Some(transcripts) = &mut live.transcripts {
                    transcripts.follow(&payload);
                }
                let events = live.hook_events(&payload);
                let effects = observe(live, &events, now, &mut conversations, &mut usage);
                if let Some(guard) = guard {
                    decide_guard(live, guard, &events, now);
                }
                effects
            }
            HolderEvent::Tap { payload } => {
                let events = live.tap_events(&payload);
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
        for sample in &usage {
            state.note_rate_limits(sample);
        }
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
            AgentEvent::TitleChanged { title } => retitle(live, title),
            _ => {}
        }
        effects.extend(live.status.observe(Observation::Agent(event.clone()), now));
    }
    effects
}

fn retitle(live: &mut Live, title: &str) {
    live.record.title = Some(title.trim())
        .filter(|title| !title.is_empty())
        .map(Into::into);
}

fn decide_guard(live: &mut Live, guard: u64, events: &[AgentEvent], now: Instant) {
    let check = guard_check(live.capabilities(), events);
    let agent_dirs = live.agent_dirs();
    let decision = check.map(|check| {
        let context = GuardContext {
            worktree: &live.record.worktree,
            branch: &live.record.branch,
            base_branch: &live.record.base,
            enabled: live.record.guards_enabled,
            allowed: &live.record.guard_allowances,
            agent_dirs: &agent_dirs,
        };
        let decision = evaluate_guard(check.action, check.cwd, &context);
        (check.tool.to_owned(), decision)
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

struct GuardCheck<'a> {
    tool: &'a str,
    action: &'a GuardedAction,
    cwd: Option<&'a Path>,
}

fn guard_check(capabilities: Capabilities, events: &[AgentEvent]) -> Option<GuardCheck<'_>> {
    if !capabilities.guards_available() {
        return None;
    }
    events.iter().find_map(|event| match event {
        AgentEvent::GuardCheck { tool, action, cwd } => Some(GuardCheck {
            tool,
            action,
            cwd: cwd.as_deref().map(Path::new),
        }),
        _ => None,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn check() -> Vec<AgentEvent> {
        vec![AgentEvent::GuardCheck {
            tool: "Bash".into(),
            action: GuardedAction::Shell {
                command: "ls".into(),
            },
            cwd: None,
        }]
    }

    #[test]
    fn guard_checks_are_evaluated_when_the_agent_has_hooks_and_guards() {
        let capabilities = Capabilities {
            hooks: true,
            guards: true,
            ..Capabilities::default()
        };
        assert!(guard_check(capabilities, &check()).is_some());
    }

    #[test]
    fn guard_checks_are_ignored_for_an_agent_without_guards() {
        let capabilities = Capabilities {
            hooks: true,
            ..Capabilities::default()
        };
        assert!(guard_check(capabilities, &check()).is_none());
    }
}
