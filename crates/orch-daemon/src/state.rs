use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use orch_agent::{GuardHit, GuardKind, mode_name};
use orch_core::{Phase, PhaseEvent, SessionId, SessionStatus};
use orch_holder::{Size, ToHolder};
use orch_protocol::{FromDaemon, GuardKindView, GuardPrompt, SessionView, SubagentView};
use orch_store::SessionRecord;
use tokio::sync::mpsc::Sender;
use tokio::sync::{Notify, watch};

use crate::DaemonConfig;
use crate::agents::Adapter;
use crate::outbox::Outbox;
use crate::store::StoreHandle;

const IDLE_CHECK: Duration = Duration::from_millis(100);
const STALL_CHECK: Duration = Duration::from_secs(1);

pub(crate) type ClientId = u64;
pub(crate) type PaneId = u64;

pub(crate) struct Daemon {
    pub(crate) config: DaemonConfig,
    pub(crate) store: StoreHandle,
    state: Mutex<State>,
    pub(crate) shutdown: Notify,
}

pub(crate) struct State {
    store: StoreHandle,
    pub(crate) sessions: HashMap<SessionId, Live>,
    clients: HashMap<ClientId, ClientLink>,
    next_id: u64,
    pub(crate) busy: usize,
}

struct ClientLink {
    outbox: Arc<Outbox>,
    visible: Option<SessionId>,
    focused: bool,
}

pub(crate) struct Live {
    pub(crate) record: SessionRecord,
    pub(crate) repo: PathBuf,
    pub(crate) status: SessionStatus,
    pub(crate) adapter: Adapter,
    pub(crate) prompts: Vec<PendingGuard>,
    pub(crate) setup_output: Option<String>,
    pub(crate) last_error: Option<String>,
    pub(crate) panes: PaneSizes,
    holder: Option<HolderLink>,
    generation: u64,
}

pub(crate) struct HolderLink {
    pub(crate) outbox: Sender<ToHolder>,
    pub(crate) pid: u32,
    pub(crate) closed: watch::Receiver<bool>,
}

pub(crate) struct PendingGuard {
    pub(crate) id: u64,
    pub(crate) tool: String,
    pub(crate) hit: GuardHit,
}

#[derive(Default)]
pub(crate) struct PaneSizes {
    open: Vec<(PaneId, Size)>,
    applied: Option<Size>,
}

impl PaneSizes {
    pub(crate) fn touch(&mut self, pane: PaneId, size: Option<Size>) {
        let known = self
            .open
            .iter()
            .position(|(open, _)| *open == pane)
            .map(|at| self.open.remove(at).1);
        if let Some(size) = size.or(known) {
            self.open.push((pane, size));
        }
    }

    pub(crate) fn close(&mut self, pane: PaneId) {
        self.open.retain(|(open, _)| *open != pane);
    }

    pub(crate) fn wanted(&self) -> Option<Size> {
        self.open.last().map(|(_, size)| *size)
    }

    fn unapplied(&self) -> Option<Size> {
        self.wanted().filter(|size| self.applied != Some(*size))
    }
}

impl Live {
    pub(crate) fn new(record: SessionRecord, repo: PathBuf, adapter: Adapter) -> Self {
        let mut status = adapter.capabilities().session_status();
        status.restore(record.phase, record.flags.clone());
        Self {
            record,
            repo,
            status,
            adapter,
            prompts: Vec::new(),
            setup_output: None,
            last_error: None,
            panes: PaneSizes::default(),
            holder: None,
            generation: 0,
        }
    }

    pub(crate) fn transition(&mut self, event: PhaseEvent) -> Result<(), String> {
        self.status.transition(event).map(drop).map_err(|refused| {
            format!(
                "not possible while the Session is {}",
                phase_name(refused.from)
            )
        })
    }

    pub(crate) fn replace_holder(&mut self, link: Option<HolderLink>) -> (u64, Option<HolderLink>) {
        self.generation += 1;
        self.panes.applied = None;
        if link.is_none() {
            self.prompts.clear();
        }
        let previous = std::mem::replace(&mut self.holder, link);
        (self.generation, previous)
    }

    pub(crate) fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
    }

    pub(crate) fn has_holder(&self) -> bool {
        self.holder.is_some()
    }

    pub(crate) fn holder_outbox(&self) -> Option<Sender<ToHolder>> {
        self.holder.as_ref().map(|link| link.outbox.clone())
    }

    pub(crate) fn send_to_holder(&self, message: ToHolder) -> bool {
        self.holder
            .as_ref()
            .is_some_and(|link| link.outbox.try_send(message).is_ok())
    }

    pub(crate) fn apply_pane_size(&mut self) {
        if let Some(size) = self.panes.unapplied()
            && self.send_to_holder(ToHolder::Resize(size))
        {
            self.panes.applied = Some(size);
        }
    }

    fn sync_record(&mut self) {
        let record = &mut self.record;
        record.phase = self.status.phase();
        record.flags = self.status.flags().clone();
        record.last_mode = self.status.permission_mode().or(record.last_mode);
        record.agent_state = self.status.agent_state().or(record.agent_state);
    }

    fn view(&self) -> SessionView {
        let record = &self.record;
        let usage = self.status.usage();
        SessionView {
            id: record.id.clone(),
            repo: self.repo.clone(),
            slug: record.slug.clone(),
            branch: record.branch.clone(),
            base: record.base.clone(),
            worktree: record.worktree.clone(),
            phase: self.status.phase().into(),
            agent: self.status.agent_state().map(Into::into),
            flags: self.status.flags().into(),
            preset: record.preset.clone(),
            mode: record.last_mode.map(|mode| mode_name(mode).into()),
            conversation: record.latest_conversation().map(|id| id.as_str().into()),
            port_base: record.port_block.map(|block| block.base),
            port_size: record.port_block.map(|block| block.size),
            setup_output: self.setup_output.clone(),
            error: self.last_error.clone(),
            holder_pid: self.holder.as_ref().map(|link| link.pid),
            guards_enabled: record.guards_enabled,
            guard_prompts: self
                .prompts
                .iter()
                .map(|pending| GuardPrompt {
                    id: pending.id,
                    tool: pending.tool.clone(),
                    kind: guard_kind_view(pending.hit.kind),
                    target: pending.hit.target.clone(),
                })
                .collect(),
            context_used_percent: usage.and_then(|usage| usage.context_used_percent),
            cost_usd: usage.and_then(|usage| usage.cost_usd),
            subagents: self
                .status
                .subagents()
                .iter()
                .map(|subagent| SubagentView {
                    id: subagent.id.as_str().into(),
                    agent_type: subagent.agent_type.clone(),
                    description: subagent.description.clone(),
                    tool_count: subagent.tool_count,
                    done: subagent.done,
                })
                .collect(),
        }
    }
}

fn guard_kind_view(kind: GuardKind) -> GuardKindView {
    match kind {
        GuardKind::BaseBranch => GuardKindView::BaseBranch,
        GuardKind::OtherRef => GuardKindView::OtherRef,
        GuardKind::WorktreeManagement => GuardKindView::WorktreeManagement,
        GuardKind::WriteOutsideWorktree => GuardKindView::WriteOutsideWorktree,
    }
}

fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::SettingUp => "Setting up",
        Phase::SetupFailed => "Setup failed",
        Phase::Active => "Active",
        Phase::PrOpen => "PR open",
        Phase::Suspended => "Suspended",
        Phase::Landed => "Landed",
        Phase::Discarded => "Discarded",
    }
}

impl Daemon {
    pub(crate) fn new(config: DaemonConfig, store: StoreHandle) -> Self {
        Self {
            config,
            state: Mutex::new(State {
                store: store.clone(),
                sessions: HashMap::new(),
                clients: HashMap::new(),
                next_id: 0,
                busy: 0,
            }),
            store,
            shutdown: Notify::new(),
        }
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn session_dir(&self, id: &SessionId) -> PathBuf {
        self.config.sessions_dir.join(id.as_str())
    }

    pub(crate) async fn watch_idle(self: Arc<Self>) {
        let mut idle_since: Option<Instant> = None;
        loop {
            tokio::time::sleep(IDLE_CHECK).await;
            if self.lock().needed() {
                idle_since = None;
                continue;
            }
            let since = *idle_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= self.config.idle_timeout {
                self.shutdown.notify_one();
                return;
            }
        }
    }

    pub(crate) async fn tick_stalled(self: Arc<Self>) {
        loop {
            tokio::time::sleep(STALL_CHECK).await;
            let loader = self.config.loader.clone();
            let stalled_after = tokio::task::spawn_blocking(move || loader.global())
                .await
                .ok()
                .and_then(Result::ok)
                .map_or(orch_core::DEFAULT_STALLED_AFTER, |global| {
                    global.stalled_after
                });
            let mut state = self.lock();
            let now = Instant::now();
            let flipped: Vec<SessionId> = state
                .sessions
                .iter_mut()
                .filter_map(|(id, live)| {
                    let before = live.status.flags().stalled;
                    live.status.tick(now, stalled_after);
                    (live.status.flags().stalled != before).then(|| id.clone())
                })
                .collect();
            for id in flipped {
                state.changed(&id);
            }
        }
    }
}

impl State {
    fn needed(&self) -> bool {
        self.busy > 0
            || !self.clients.is_empty()
            || self
                .sessions
                .values()
                .any(|live| live.has_holder() || live.status.phase() == Phase::PrOpen)
    }

    pub(crate) fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub(crate) fn add_client(&mut self, outbox: Arc<Outbox>) -> ClientId {
        let id = self.next_id();
        let sessions = self.sessions.values().map(Live::view).collect();
        outbox.send(FromDaemon::Sessions { sessions });
        self.clients.insert(
            id,
            ClientLink {
                outbox,
                visible: None,
                focused: false,
            },
        );
        id
    }

    pub(crate) fn remove_client(&mut self, id: ClientId) {
        if self.clients.remove(&id).is_some() {
            self.refresh_watched();
        }
    }

    pub(crate) fn set_view(&mut self, id: ClientId, visible: Option<SessionId>, focused: bool) {
        if let Some(client) = self.clients.get_mut(&id) {
            client.visible = visible;
            client.focused = focused;
        }
        self.refresh_watched();
    }

    fn refresh_watched(&mut self) {
        let ids: Vec<SessionId> = self.sessions.keys().cloned().collect();
        for id in ids {
            let watched = self
                .clients
                .values()
                .any(|client| client.focused && client.visible.as_ref() == Some(&id));
            let Some(live) = self.sessions.get_mut(&id) else {
                continue;
            };
            let was_unseen = live.status.flags().unseen;
            live.status.set_watched(watched);
            if live.status.flags().unseen != was_unseen {
                self.changed(&id);
            }
        }
    }

    pub(crate) fn changed(&mut self, id: &SessionId) {
        let Some(live) = self.sessions.get_mut(id) else {
            return;
        };
        live.sync_record();
        let record = live.record.clone();
        self.store.write(move |store| {
            if let Err(err) = store.save_session(&record) {
                eprintln!("orch daemon: saving Session {}: {err}", record.id.as_str());
            }
        });
        let view = Box::new(live.view());
        for client in self.clients.values() {
            client.outbox.session_changed(view.clone());
        }
    }

    pub(crate) fn insert(&mut self, live: Live) {
        let id = live.record.id.clone();
        self.sessions.insert(id.clone(), live);
        self.changed(&id);
    }
}
