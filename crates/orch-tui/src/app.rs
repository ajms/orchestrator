use std::collections::HashMap;

use crossterm::event::{Event as TermEvent, KeyEventKind};
use orch_core::SessionId;
use orch_protocol::{
    AgentStateView, CreateSession, FromDaemon, GuardChoice, GuardPrompt, LandingMode, PhaseView,
    Reply, Request, RequestError, SessionView, Size,
};

pub use crate::config::TuiConfig;
use crate::discard::DiscardConfirm;
use crate::event::{
    EditorError, Effect, Event, PaneId, RateLimits, ReviewData, ReviewPurpose, ReviewTarget,
};
use crate::guard::{GuardId, Guards};
use crate::land::LandForm;
use crate::layout::Areas;
use crate::link::RequestId;
use crate::new_form::NewForm;
use crate::pane::PaneMirror;
use crate::review::ReviewView;
use crate::sessions::{Sessions, phase_label};

pub(crate) const NO_SESSION: &str = "no Session selected";

pub(crate) enum Call {
    Request(RequestId, Request),
    OpenPane(PaneId, SessionId, Size),
    ClosePane,
    Input(Vec<u8>),
    Paste(String),
    ResizePane(Size),
    Local(Effect),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Sidebar,
    Pane,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mode {
    Normal,
    Insert,
    CommandLine(String),
    Visual(Selection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Selection {
    pub linewise: bool,
    pub anchor: (u16, u16),
    pub cursor: (u16, u16),
}

impl Selection {
    pub fn ordered(&self) -> ((u16, u16), (u16, u16)) {
        match self.anchor <= self.cursor {
            true => (self.anchor, self.cursor),
            false => (self.cursor, self.anchor),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Prefix {
    CtrlBackslash,
    CtrlW,
    G,
}

enum Pending {
    ReportFailure,
    Resume(SessionId),
    Create(CreateSession),
    Trust(CreateSession),
    Draft(SessionId, LandingMode),
    Land,
    DiscardPreview(SessionId),
}

pub(crate) struct TrustPrompt {
    pub create: CreateSession,
    pub hash: String,
    pub items: Vec<String>,
}

pub(crate) enum Popup {
    New(NewForm),
    Trust(TrustPrompt),
    Land(LandForm),
    Discard(DiscardConfirm),
}

pub(crate) struct App {
    pub config: TuiConfig,
    pub sessions: Sessions,
    pub selected: Option<SessionId>,
    pub pane: Option<PaneMirror>,
    pub focus: Focus,
    pub mode: Mode,
    pub prefix: Option<Prefix>,
    pub message: Option<String>,
    pub rate_limits: RateLimits,
    pub popup: Option<Popup>,
    pub review: Option<ReviewView>,
    pub mismatch: Option<String>,
    size: Size,
    guards: Guards,
    select_when_listed: Option<SessionId>,
    insert_when_open: Option<SessionId>,
    terminal_focused: bool,
    reported_view: Option<(Option<SessionId>, bool)>,
    next_request: u64,
    next_pane: u64,
    pending: HashMap<RequestId, Pending>,
    calls: Vec<Call>,
}

impl App {
    pub fn new(config: TuiConfig, size: Size) -> Self {
        Self {
            config,
            sessions: Sessions::default(),
            selected: None,
            pane: None,
            focus: Focus::Sidebar,
            mode: Mode::Normal,
            prefix: None,
            message: None,
            rate_limits: RateLimits::default(),
            popup: None,
            review: None,
            mismatch: None,
            size,
            guards: Guards::default(),
            select_when_listed: None,
            insert_when_open: None,
            terminal_focused: false,
            reported_view: None,
            next_request: 0,
            next_pane: 0,
            pending: HashMap::new(),
            calls: Vec::new(),
        }
    }

    pub fn update(&mut self, event: Event) -> Vec<Call> {
        match event {
            Event::Daemon(message) => self.daemon(message),
            Event::Pane { pane, message } => self.pane_message(pane, message),
            Event::Terminal(event) => self.terminal(event),
            Event::RateLimits(limits) => self.rate_limits = limits,
            Event::EditorClosed(result) => self.editor_closed(result),
            Event::Review {
                session,
                purpose,
                result,
            } => self.review_loaded(session, purpose, result),
            Event::Ring {
                session,
                title,
                body,
            } => self.ring(&session, &title, &body),
            Event::VersionMismatch { message } => self.mismatch = Some(message),
            Event::Notice(text) => self.message = Some(text),
            Event::Disconnected { reason } => {
                self.message = Some(format!("lost the Daemon: {reason}"));
            }
        }
        std::mem::take(&mut self.calls)
    }

    fn daemon(&mut self, message: FromDaemon) {
        match message {
            FromDaemon::Sessions { sessions } => {
                self.mismatch = None;
                self.sessions.replace(sessions);
            }
            FromDaemon::SessionChanged { session } => self.sessions.upsert(*session),
            FromDaemon::Response { id, result } => {
                if let Some(pending) = self.pending.remove(&RequestId(id)) {
                    self.answered(pending, result);
                }
            }
            _ => {}
        }
        self.follow_selection();
    }

    fn answered(&mut self, pending: Pending, result: Result<Reply, RequestError>) {
        match (pending, result) {
            (Pending::Create(create), Err(RequestError::Untrusted { hash, items })) => {
                self.popup = Some(Popup::Trust(TrustPrompt {
                    create,
                    hash,
                    items,
                }));
            }
            (Pending::Create(_), Ok(Reply::Created { session })) => {
                self.select_when_listed = Some(session);
            }
            (Pending::Trust(create), Ok(_)) => self.create_session(create),
            (Pending::Draft(session, mode), result) => self.drafted(&session, mode, result),
            (Pending::Land, Ok(Reply::Landed { commit, warning })) => {
                self.message = Some(match warning {
                    Some(warning) => format!("Landed as {commit}; warning: {warning}"),
                    None => format!("Landed as {commit}"),
                });
            }
            (
                Pending::Land,
                Ok(Reply::PrOpened {
                    number,
                    committed_changes,
                }),
            ) => {
                self.message = Some(match committed_changes {
                    true => format!(
                        "opened PR #{number}; uncommitted changes were committed with the PR title as the message"
                    ),
                    false => format!("opened PR #{number}"),
                });
            }
            (Pending::Land, Err(RequestError::Conflict { paths })) => {
                let conflict = RequestError::Conflict { paths };
                self.message = Some(format!("{conflict}; the rebase was handed to the Agent"));
            }
            (
                Pending::DiscardPreview(session),
                Ok(Reply::DiscardPreview {
                    uncommitted,
                    unlanded,
                }),
            ) => {
                let confirm = DiscardConfirm::new(session, uncommitted, unlanded);
                self.popup = Some(Popup::Discard(confirm));
            }
            (Pending::Resume(session), Err(err)) => {
                if self.insert_when_open.as_ref() == Some(&session) {
                    self.insert_when_open = None;
                }
                self.message = Some(err.to_string());
            }
            (_, Err(err)) => self.message = Some(err.to_string()),
            (_, Ok(_)) => {}
        }
    }

    fn pane_message(&mut self, pane: PaneId, message: FromDaemon) {
        let Some(mirror) = self.pane.as_mut().filter(|mirror| mirror.id == pane) else {
            return;
        };
        match message {
            FromDaemon::Screen(snapshot) => mirror.restore(&snapshot),
            FromDaemon::Output { bytes } => mirror.output(&bytes),
            FromDaemon::Resized(size) => mirror.resized(size),
            FromDaemon::InputDropped { reason } => {
                self.message = Some(format!("input not sent: {reason}"));
            }
            FromDaemon::PaneClosed { reason } => {
                mirror.closed = Some(reason);
                if matches!(self.mode, Mode::Insert | Mode::Visual(_)) {
                    self.mode = Mode::Normal;
                }
            }
            _ => {}
        }
    }

    fn terminal(&mut self, event: TermEvent) {
        match event {
            TermEvent::Key(key) if key.kind != KeyEventKind::Release => {
                crate::keymap::handle(self, key)
            }
            TermEvent::Paste(text) => crate::keymap::paste(self, text),
            TermEvent::FocusGained => self.terminal_focused = true,
            TermEvent::FocusLost => self.terminal_focused = false,
            TermEvent::Resize(cols, rows) => self.resize(Size { rows, cols }),
            _ => {}
        }
        self.follow_selection();
    }

    fn resize(&mut self, size: Size) {
        let before = self.pane_size();
        self.size = size;
        let after = self.pane_size();
        if before != after && self.pane.is_some() {
            self.calls.push(Call::ResizePane(after));
        }
    }

    fn editor_closed(&mut self, result: Result<String, EditorError>) {
        match (result, &mut self.popup) {
            (Ok(text), Some(Popup::New(form))) => form.set_prompt(text.trim_end().to_string()),
            (Ok(text), Some(Popup::Land(form))) => form.edited_text(&text),
            (Err(err), _) => self.message = Some(format!("editor failed: {err}")),
            _ => {}
        }
    }

    fn ring(&mut self, session: &SessionId, title: &str, body: &str) {
        let muted = self
            .sessions
            .get(session)
            .is_some_and(|view| view.flags.muted);
        if !muted {
            let bytes = orch_notify::terminal_attention(title, body);
            self.push(Call::Local(Effect::WriteTerminal(bytes)));
        }
    }

    pub fn pane_size(&self) -> Size {
        Areas::for_size(self.size).pane_inner()
    }

    pub fn inserting(&self) -> bool {
        self.mode == Mode::Insert
    }

    pub fn selected_view(&self) -> Option<&SessionView> {
        self.selected.as_ref().and_then(|id| self.sessions.get(id))
    }

    pub fn guard_prompt(&self) -> Option<(&SessionId, &GuardPrompt)> {
        let view = self.selected_view()?;
        self.guards.shown(view).map(|prompt| (&view.id, prompt))
    }

    pub fn guard_waiting_hidden(&self) -> bool {
        self.selected_view()
            .is_some_and(|view| self.guards.waiting_hidden(view))
    }

    pub fn hide_guard(&mut self, guard: GuardId) {
        self.guards.hide(guard);
    }

    pub fn answer_guard(&mut self, guard: GuardId, choice: GuardChoice) {
        let request = Request::AnswerGuard {
            session: guard.session.clone(),
            guard: guard.guard,
            choice,
        };
        self.guards.answer(guard);
        self.request(request, Pending::ReportFailure);
    }

    pub fn reveal_guards(&mut self) {
        if let Some(session) = self.selected.clone() {
            self.guards.reveal(&session);
        }
    }

    pub fn select_offset(&mut self, offset: isize) {
        let order = self.sessions.ordered_ids();
        let Some(at) = self
            .selected
            .as_ref()
            .and_then(|id| order.iter().position(|known| known == id))
        else {
            return;
        };
        let next = at
            .saturating_add_signed(offset)
            .min(order.len().saturating_sub(1));
        if next != at {
            self.selected = order.get(next).cloned();
            self.reveal_guards();
        }
        self.follow_selection();
    }

    pub fn push(&mut self, call: Call) {
        self.calls.push(call);
    }

    pub fn report(&mut self, request: Request) {
        self.request(request, Pending::ReportFailure);
    }

    fn request(&mut self, request: Request, pending: Pending) {
        self.next_request += 1;
        let id = RequestId(self.next_request);
        self.pending.insert(id, pending);
        self.calls.push(Call::Request(id, request));
    }

    pub fn on_selected(&mut self, request: impl FnOnce(SessionId) -> Request) {
        match self.selected.clone() {
            Some(session) => self.report(request(session)),
            None => self.message = Some(NO_SESSION.into()),
        }
    }

    pub fn resume_selected(&mut self, then_insert: bool) {
        let Some(session) = self.selected.clone() else {
            self.message = Some(NO_SESSION.into());
            return;
        };
        if then_insert {
            self.insert_when_open = Some(session.clone());
        }
        let request = Request::Resume {
            session: session.clone(),
        };
        self.request(request, Pending::Resume(session));
    }

    pub fn create_session(&mut self, create: CreateSession) {
        self.request(
            Request::CreateSession(create.clone()),
            Pending::Create(create),
        );
    }

    pub fn approve_trust(&mut self, prompt: TrustPrompt) {
        let request = Request::ApproveTrust {
            repo: prompt.create.repo.clone(),
            hash: prompt.hash,
        };
        self.request(request, Pending::Trust(prompt.create));
    }

    pub fn open_new_form(&mut self) {
        let mut repos = self.config.repos.clone();
        for repo in self.sessions.repos() {
            if !repos.iter().any(|known| known == repo) {
                repos.push(repo.to_path_buf());
            }
        }
        let preselect = match &self.config.cwd_repo {
            Some(cwd) => match repos.iter().position(|repo| repo == cwd) {
                Some(at) => at,
                None => {
                    repos.insert(0, cwd.clone());
                    0
                }
            },
            None => 0,
        };
        let presets = self.config.presets.names().map(String::from).collect();
        let form = NewForm::new(repos, preselect, presets, &self.config.branch_prefix);
        self.popup = Some(Popup::New(form));
        self.refresh_base_candidates();
    }

    pub fn refresh_base_candidates(&mut self) {
        let Some(Popup::New(form)) = &mut self.popup else {
            return;
        };
        let candidates = match form.repo_path() {
            Some(repo) => self
                .sessions
                .in_repo(&repo)
                .map(|view| view.branch.clone())
                .collect(),
            None => Vec::new(),
        };
        form.set_base_candidates(candidates);
    }

    pub fn open_land(&mut self) {
        let Some(view) = self.selected_view() else {
            self.message = Some(NO_SESSION.into());
            return;
        };
        if let Some(refusal) = landing_refusal(view) {
            self.message = Some(refusal);
            return;
        }
        let session = view.id.clone();
        self.popup = Some(Popup::Land(LandForm::new(session.clone())));
        self.request_draft(session, LandingMode::Squash);
    }

    pub fn request_draft(&mut self, session: SessionId, mode: LandingMode) {
        let request = Request::Draft {
            session: session.clone(),
            mode,
        };
        self.request(request, Pending::Draft(session, mode));
    }

    fn drafted(
        &mut self,
        session: &SessionId,
        mode: LandingMode,
        result: Result<Reply, RequestError>,
    ) {
        let Some(Popup::Land(form)) = &mut self.popup else {
            return;
        };
        if &form.session != session {
            return;
        }
        match result {
            Ok(Reply::Drafted { title, body }) => form.drafted(mode, title, &body),
            Ok(_) => form.draft_failed(),
            Err(err) => {
                form.draft_failed();
                self.message = Some(format!("no draft: {err}"));
            }
        }
    }

    pub fn land(&mut self, session: SessionId, landing: orch_protocol::Landing) {
        self.message = Some("Landing…".into());
        self.request(Request::Land { session, landing }, Pending::Land);
    }

    pub fn load_discard_preview(&mut self) {
        let Some(session) = self.selected.clone() else {
            self.message = Some(NO_SESSION.into());
            return;
        };
        let request = Request::DiscardPreview {
            session: session.clone(),
        };
        self.request(request, Pending::DiscardPreview(session));
    }

    pub fn load_review(&mut self, purpose: ReviewPurpose) {
        let Some(view) = self.selected_view() else {
            self.message = Some(NO_SESSION.into());
            return;
        };
        let effect = Effect::LoadReview {
            session: view.id.clone(),
            target: ReviewTarget {
                repo: view.repo.clone(),
                worktree: view.worktree.clone(),
                slug: view.slug.clone(),
                branch: view.branch.clone(),
                base: view.base.clone(),
            },
            purpose,
        };
        self.message = Some(format!("diffing {} against {}…", view.slug, view.base));
        self.push(Call::Local(effect));
    }

    fn review_loaded(
        &mut self,
        session: SessionId,
        purpose: ReviewPurpose,
        result: Result<ReviewData, orch_git::Error>,
    ) {
        let Some(view) = self.sessions.get(&session) else {
            return;
        };
        let data = match result {
            Ok(data) => data,
            Err(err) => {
                self.message = Some(format!("Review failed: {err}"));
                return;
            }
        };
        self.message = None;
        match purpose {
            ReviewPurpose::BuiltIn => {
                self.review = Some(ReviewView::new(view.base.clone(), data.files));
            }
            ReviewPurpose::External => {
                let effect = Effect::RunExternal {
                    command: self.config.review_command.clone(),
                    cwd: view.worktree.clone(),
                    env: vec![
                        ("ORCH_BASE".into(), view.base.clone()),
                        ("ORCH_MERGE_BASE".into(), data.merge_base),
                        ("ORCH_REVIEW_TREE".into(), data.tree),
                    ],
                };
                self.push(Call::Local(effect));
            }
        }
    }

    fn follow_selection(&mut self) {
        let order = self.sessions.ordered_ids();
        if let Some(wanted) = self
            .select_when_listed
            .take_if(|wanted| order.contains(wanted))
        {
            self.selected = Some(wanted);
        }
        if !self.selected.as_ref().is_some_and(|id| order.contains(id)) {
            self.selected = order.first().cloned();
        }
        self.sync_pane();
        self.report_view();
    }

    fn sync_pane(&mut self) {
        let wanted = self
            .selected_view()
            .filter(|view| view.phase.is_live())
            .map(|view| (view.id.clone(), view.holder_pid));
        let current = self
            .pane
            .as_ref()
            .map(|pane| (pane.session.clone(), pane.holder_pid));
        if wanted == current {
            return;
        }
        if self.pane.take().is_some() {
            self.calls.push(Call::ClosePane);
        }
        let Some((session, holder_pid)) = wanted else {
            return;
        };
        self.next_pane += 1;
        let id = PaneId(self.next_pane);
        let size = self.pane_size();
        self.pane = Some(PaneMirror::new(id, session.clone(), holder_pid, size));
        self.calls.push(Call::OpenPane(id, session.clone(), size));
        if self
            .insert_when_open
            .take_if(|wanted| *wanted == session)
            .is_some()
        {
            self.focus = Focus::Pane;
            self.mode = Mode::Insert;
        }
    }

    fn report_view(&mut self) {
        let view = (self.selected.clone(), self.terminal_focused);
        if self.reported_view.as_ref() == Some(&view) {
            return;
        }
        self.reported_view = Some(view.clone());
        let (session, focused) = view;
        self.report(Request::View { session, focused });
    }
}

fn landing_refusal(view: &SessionView) -> Option<String> {
    if view.phase == PhaseView::Suspended {
        return None;
    }
    if !view.phase.is_live() {
        return Some(format!(
            "Landing is blocked while {}",
            phase_label(view.phase)
        ));
    }
    match view.agent {
        Some(AgentStateView::Idle | AgentStateView::Exited | AgentStateView::Errored) => None,
        Some(AgentStateView::Working) => Some("Landing is blocked: the Agent is Working".into()),
        Some(AgentStateView::NeedsInput) => {
            Some("Landing is blocked: the Agent Needs input".into())
        }
        _ => Some("Landing is blocked until the Agent settles".into()),
    }
}
