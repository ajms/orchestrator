use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use orch_agent::{GuardHit, GuardKind, SubagentTranscripts, mode_name};
use orch_core::{AgentState, GateRefusal, Phase, PhaseEvent, PrStatus, SessionId, SessionStatus};
use orch_git::{SessionName, SessionWorktree};
use orch_holder::{Size, ToHolder};
use orch_notify::{AttentionEvent, ClientView};
use orch_protocol::{
    DisplayVars, FlagsView, FromDaemon, GuardKindView, GuardPrompt, ReconcileReport, SessionView,
    SubagentView,
};
use orch_store::SessionRecord;
use tokio::sync::mpsc::Sender;
use tokio::sync::{Notify, watch};

use crate::DaemonConfig;
use crate::agents::Adapter;
use crate::notify::Notifier;
use crate::outbox::Outbox;
use crate::rate_limits::RateLimits;
use crate::recency::{LastUsed, UseClock};
use crate::store::StoreHandle;
use crate::transcript::Following;

const IDLE_CHECK: Duration = Duration::from_millis(100);
const STALL_CHECK: Duration = Duration::from_secs(1);

pub(crate) type ClientId = u64;
pub(crate) type PaneId = u64;

pub(crate) struct Daemon {
    pub(crate) config: DaemonConfig,
    pub(crate) store: StoreHandle,
    state: Mutex<State>,
    pub(crate) shutdown: Notify,
    pub(crate) reconciling: tokio::sync::Mutex<()>,
    repo_guards: Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
}

pub(crate) struct State {
    store: StoreHandle,
    pub(crate) sessions: HashMap<SessionId, Live>,
    clients: HashMap<ClientId, ClientLink>,
    next_id: u64,
    pub(crate) busy: usize,
    pub(crate) report: Option<ReconcileReport>,
    pub(crate) passes: Passes,
    pub(crate) display: DisplayVars,
    notifier: Notifier,
    rate_limits: RateLimits,
    use_clock: UseClock,
}

#[derive(Default)]
pub(crate) struct Passes {
    pub(crate) requested: u64,
    pub(crate) full_requested: u64,
    pub(crate) completed: u64,
    pub(crate) full_completed: u64,
}

struct ClientLink {
    outbox: Arc<Outbox>,
    session_in_view: Option<SessionId>,
    focused: bool,
    last_used: LastUsed,
    peer: Option<i32>,
    following: Option<Following>,
}

impl ClientLink {
    fn view(&self, id: ClientId) -> ClientView {
        ClientView {
            id: orch_notify::ClientId(id),
            focused: self.focused,
            session_in_view: self.session_in_view.clone(),
        }
    }
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
    pub(crate) exclusive: bool,
    pub(crate) comments: CommentCursor,
    pub(crate) recheck: Recheck,
    pub(crate) launching: bool,
    pub(crate) repo_missing: bool,
    pub(crate) transcripts: Option<Box<dyn SubagentTranscripts>>,
    end_noticed: bool,
    holder: Option<HolderLink>,
    generation: u64,
}

#[derive(Default)]
pub(crate) struct CommentCursor {
    seen: Option<u32>,
    total: u32,
}

impl CommentCursor {
    pub(crate) fn opened() -> Self {
        Self {
            seen: Some(0),
            total: 0,
        }
    }

    fn unseen(&mut self, total: u32, persisted_unseen: u32, watched: bool) -> u32 {
        let seen = self
            .seen
            .get_or_insert(total.saturating_sub(persisted_unseen));
        if watched {
            *seen = total;
        }
        self.total = total;
        total.saturating_sub(*seen)
    }

    fn mark_seen(&mut self) {
        if self.seen.is_some() {
            self.seen = Some(self.total);
        }
    }
}

#[derive(Default)]
pub(crate) struct Recheck {
    pub(crate) running: bool,
    pub(crate) again: bool,
}

pub(crate) struct HolderLink {
    pub(crate) outbox: Sender<ToHolder>,
    pub(crate) pid: u32,
    pub(crate) port_block: Option<orch_store::PortBlock>,
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
        let capabilities = adapter.capabilities();
        let mut status = capabilities.session_status();
        status.restore(record.phase, record.flags.clone());
        let transcripts = capabilities
            .transcripts
            .then(|| adapter.subagent_transcripts())
            .flatten();
        Self {
            record,
            repo,
            status,
            adapter,
            prompts: Vec::new(),
            setup_output: None,
            last_error: None,
            panes: PaneSizes::default(),
            exclusive: false,
            comments: CommentCursor::default(),
            recheck: Recheck::default(),
            launching: false,
            repo_missing: false,
            transcripts,
            end_noticed: false,
            holder: None,
            generation: 0,
        }
    }

    pub(crate) fn transition(&mut self, event: PhaseEvent) -> Result<(), String> {
        self.status.transition(event).map_err(|refused| {
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

    pub(crate) fn waits_on_pr(&self) -> bool {
        self.status.flags().pr.is_some()
            && matches!(self.status.phase(), Phase::PrOpen | Phase::Suspended)
    }

    pub(crate) fn holder_port_block(&self) -> Option<orch_store::PortBlock> {
        self.holder.as_ref().and_then(|link| link.port_block)
    }

    pub(crate) fn relocate(&mut self, repo: PathBuf, worktree: PathBuf) {
        self.repo = repo;
        self.record.worktree = worktree;
        self.repo_missing = false;
    }

    pub(crate) fn is_running(&self) -> bool {
        self.has_holder() || self.launching
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

    pub(crate) fn update_pr(&mut self, mut pr: PrStatus, comments: u32) {
        let unseen = self
            .status
            .flags()
            .pr
            .as_ref()
            .map_or(0, |pr| pr.new_comments);
        pr.new_comments = self
            .comments
            .unseen(comments, unseen, self.status.is_watched());
        self.status.update_pr(pr)
    }

    fn set_watched(&mut self, watched: bool) {
        self.status.set_watched(watched);
        if watched
            && let Some(pr) = self.status.flags().pr.clone()
            && pr.new_comments > 0
        {
            self.comments.mark_seen();
            self.status.update_pr(PrStatus {
                new_comments: 0,
                ..pr
            });
        }
    }

    pub(crate) fn hand_back(&mut self, prompt: String) {
        self.record.queued_prompt = Some(prompt);
        self.deliver_prompt();
    }

    pub(crate) fn deliver_prompt(&mut self) -> bool {
        let ready = self.status.agent_state() == Some(AgentState::Idle)
            && self.has_holder()
            && !self.exclusive;
        let Some(prompt) = self.record.queued_prompt.take_if(|_| ready) else {
            return false;
        };
        self.send_to_holder(ToHolder::Paste { text: prompt });
        self.send_to_holder(ToHolder::Input {
            bytes: b"\r".to_vec(),
        });
        true
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
            title: record.title.clone(),
            branch: record.branch.clone(),
            base: record.base.clone(),
            worktree: record.worktree.clone(),
            phase: self.status.phase().into(),
            agent: self.status.agent_state().map(Into::into),
            flags: FlagsView::new(self.status.flags(), self.repo_missing),
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
        GuardKind::ExternalTool => GuardKindView::ExternalTool,
    }
}

pub(crate) fn session_worktree(record: &SessionRecord) -> SessionWorktree {
    SessionWorktree {
        path: record.worktree.clone(),
        name: SessionName {
            slug: record.slug.clone(),
            branch: record.branch.clone(),
        },
        base: record.base.clone(),
    }
}

pub(crate) fn gate_message(action: &str, refusal: GateRefusal) -> String {
    match refusal {
        GateRefusal::WrongPhase { phase } => format!(
            "{action} is not possible while the Session is {}",
            phase_name(phase)
        ),
        GateRefusal::AgentBusy { state } => format!(
            "{action} needs the Agent to be Idle, Exited or Errored; it is {}",
            state.map_or("not running", agent_state_name)
        ),
    }
}

fn agent_state_name(state: AgentState) -> &'static str {
    match state {
        AgentState::Starting => "Starting",
        AgentState::Working => "Working",
        AgentState::NeedsInput => "Needs input",
        AgentState::Idle => "Idle",
        AgentState::Errored => "Errored",
        AgentState::Exited => "Exited",
        AgentState::Unknown => "in an unknown state",
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
    pub(crate) fn new(
        config: DaemonConfig,
        store: StoreHandle,
        clicks: tokio::sync::mpsc::Sender<SessionId>,
    ) -> Self {
        let notifier = Notifier::spawn(
            config.notifications.clone(),
            config.loader.clone(),
            move |session| {
                if clicks.try_send(session).is_err() {
                    eprintln!("orch daemon: a notification click was dropped");
                }
            },
        );
        Self {
            config,
            state: Mutex::new(State {
                store: store.clone(),
                sessions: HashMap::new(),
                clients: HashMap::new(),
                next_id: 0,
                busy: 0,
                report: None,
                passes: Passes::default(),
                display: DisplayVars::default(),
                notifier,
                rate_limits: RateLimits::default(),
                use_clock: UseClock::default(),
            }),
            store,
            shutdown: Notify::new(),
            reconciling: tokio::sync::Mutex::new(()),
            repo_guards: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) async fn repo_guard(
        &self,
        repo: &std::path::Path,
    ) -> tokio::sync::OwnedMutexGuard<()> {
        let guard = self
            .repo_guards
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(repo.to_path_buf())
            .or_default()
            .clone();
        guard.lock_owned().await
    }

    pub(crate) async fn repo_guards<const N: usize>(
        &self,
        repos: [&std::path::Path; N],
    ) -> Vec<tokio::sync::OwnedMutexGuard<()>> {
        let mut repos = repos.to_vec();
        repos.sort();
        repos.dedup();
        let mut guards = Vec::new();
        for repo in repos {
            guards.push(self.repo_guard(repo).await);
        }
        guards
    }

    pub(crate) fn session_dir(&self, id: &SessionId) -> PathBuf {
        self.config.sessions_dir.join(id.as_str())
    }

    pub(crate) async fn route_clicks(
        self: Arc<Self>,
        mut clicked: tokio::sync::mpsc::Receiver<SessionId>,
    ) {
        while let Some(session) = clicked.recv().await {
            self.lock().focus_recent_client(session);
        }
    }

    pub(crate) async fn watch_idle(self: Arc<Self>) {
        let Some(idle_timeout) = self.config.idle_timeout else {
            return;
        };
        let mut idle_since: Option<Instant> = None;
        loop {
            tokio::time::sleep(IDLE_CHECK).await;
            if self.lock().needed() {
                idle_since = None;
                continue;
            }
            let since = *idle_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= idle_timeout {
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
                .any(|live| live.has_holder() || live.waits_on_pr())
    }

    pub(crate) fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub(crate) fn add_client(
        &mut self,
        outbox: Arc<Outbox>,
        peer: Option<i32>,
    ) -> (ClientId, LastUsed) {
        let id = self.next_id();
        let sessions = self.sessions.values().map(Live::view).collect();
        outbox.send(FromDaemon::Sessions { sessions });
        if self.rate_limits.is_known() {
            outbox.send(self.rate_limits.into());
        }
        if let Some(report) = &self.report {
            outbox.send(FromDaemon::Reconciled {
                report: Box::new(report.clone()),
            });
        }
        let link = ClientLink {
            outbox,
            session_in_view: None,
            focused: false,
            last_used: self.use_clock.stamp(),
            peer,
            following: None,
        };
        let last_used = link.last_used.clone();
        self.notifier
            .client_view(link.view(id), link.outbox.clone());
        self.clients.insert(id, link);
        (id, last_used)
    }

    pub(crate) fn remove_client(&mut self, id: ClientId) {
        if self.clients.remove(&id).is_some() {
            self.notifier.client_gone(orch_notify::ClientId(id));
            self.refresh_watched();
        }
    }

    pub(crate) fn set_view(
        &mut self,
        id: ClientId,
        session_in_view: Option<SessionId>,
        focused: bool,
    ) {
        if let Some(client) = self.clients.get_mut(&id) {
            client.session_in_view = session_in_view;
            client.focused = focused;
            self.notifier
                .client_view(client.view(id), client.outbox.clone());
        }
        self.refresh_watched();
    }

    pub(crate) fn client_outbox(&self, id: ClientId) -> Option<Arc<Outbox>> {
        self.clients.get(&id).map(|client| client.outbox.clone())
    }

    pub(crate) fn follow(&mut self, id: ClientId, following: Option<Following>) {
        if let Some(client) = self.clients.get_mut(&id) {
            client.following = following;
        }
    }

    pub(crate) fn client_using(&self, peer: Option<i32>, session: &SessionId) -> Option<LastUsed> {
        self.clients
            .values()
            .filter(|client| client.session_in_view.as_ref() == Some(session))
            .filter(|client| peer.is_none() || client.peer == peer)
            .max_by_key(|client| client.last_used.at())
            .map(|client| client.last_used.clone())
    }

    pub(crate) fn relay_copy(&self, session: &SessionId, text: String) {
        for client in self.clients.values() {
            if client.session_in_view.as_ref() == Some(session) {
                client.outbox.send(FromDaemon::Clipboard {
                    session: session.clone(),
                    text: text.clone(),
                });
            }
        }
    }

    fn focus_recent_client(&self, session: SessionId) {
        if let Some(client) = self
            .clients
            .values()
            .max_by_key(|client| client.last_used.at())
        {
            client.outbox.send(FromDaemon::Focus { session });
        }
    }

    pub(crate) fn note_rate_limits(&mut self, sample: &orch_core::UsageSample) {
        let latest = self.rate_limits.after(sample);
        if latest == self.rate_limits {
            return;
        }
        self.rate_limits = latest;
        for client in self.clients.values() {
            client.outbox.send(latest.into());
        }
    }

    pub(crate) fn dismiss(&self, id: &SessionId) {
        self.notifier.dismiss(id.clone());
    }

    fn forward_attention(&mut self, id: &SessionId) {
        let Some(live) = self.sessions.get_mut(id) else {
            return;
        };
        let raised = live.status.take_attention();
        let ended = live.status.phase().is_terminal() && !live.end_noticed;
        live.end_noticed |= ended;
        if ended && raised.is_empty() {
            self.notifier.dismiss(id.clone());
        }
        let now = Instant::now();
        let repo_name = live
            .repo
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        for attention in raised {
            let event = AttentionEvent {
                session: id.clone(),
                title: live
                    .record
                    .title
                    .clone()
                    .unwrap_or_else(|| live.record.slug.clone()),
                repo: repo_name.clone(),
                branch: live.record.branch.clone(),
                attention,
                at: now,
            };
            self.notifier
                .attention(event, live.repo.clone(), live.status.flags().muted);
        }
    }

    fn refresh_watched(&mut self) {
        let ids: Vec<SessionId> = self.sessions.keys().cloned().collect();
        for id in ids {
            let watched = self
                .clients
                .values()
                .any(|client| client.focused && client.session_in_view.as_ref() == Some(&id));
            let Some(live) = self.sessions.get_mut(&id) else {
                continue;
            };
            if watched && !live.status.is_watched() {
                self.notifier.dismiss(id.clone());
            }
            let before = live.status.flags().clone();
            live.set_watched(watched);
            if *live.status.flags() != before {
                self.changed(&id);
            }
        }
    }

    pub(crate) fn changed(&mut self, id: &SessionId) {
        self.forward_attention(id);
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

    pub(crate) fn remove_session(&mut self, id: &SessionId) {
        if self.sessions.remove(id).is_none() {
            return;
        }
        for client in self.clients.values() {
            client.outbox.session_removed(id.clone());
        }
    }

    pub(crate) fn publish_report(&mut self, report: &ReconcileReport) {
        if self.report.as_ref() == Some(report) {
            return;
        }
        self.report = Some(report.clone());
        for client in self.clients.values() {
            client.outbox.send(FromDaemon::Reconciled {
                report: Box::new(report.clone()),
            });
        }
    }

    pub(crate) fn insert(&mut self, live: Live) {
        let id = live.record.id.clone();
        self.sessions.insert(id.clone(), live);
        self.changed(&id);
    }
}
