use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use crossterm::event::{Event as TermEvent, KeyEventKind};
use orch_core::SessionId;
use orch_protocol::{
    AgentStateView, CreateSession, Fix, FromDaemon, GuardChoice, GuardPrompt, LandingMode,
    LeftoverView, PhaseView, ReconcileReport, Reply, Request, RequestError, SessionView, Size,
    UsageReport,
};

use crate::clipboard::copy_effects;
pub use crate::config::TuiConfig;
use crate::discard::{DiscardConfirm, DiscardTarget};
use crate::event::{EditorError, Effect, Event, PaneId, ReviewData, ReviewPurpose, ReviewTarget};
use crate::guard::{GuardId, Guards};
use crate::hyperlinks::Hyperlinks;
use crate::land::LandForm;
use crate::layout::Areas;
use crate::link::RequestId;
use crate::mouse::Region;
use crate::new_form::NewForm;
use crate::pane::PaneMirror;
use crate::reconcile::{FixStep, ReconcileView, RetargetPicker, plan};
use crate::review::{EditorTarget, ReviewAction, ReviewView};
use crate::selection::Selector;
use crate::sessions::{Sessions, phase_label, repo_name};
use crate::sidebar::{Row, SidebarView, Stop, Viewport};

pub(crate) const NO_SESSION: &str = "no Session selected";

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct RateLimits {
    pub five_hour: Option<f64>,
    pub seven_day: Option<f64>,
}

pub(crate) enum Call {
    Request(RequestId, Request),
    OpenPane(PaneId, SessionId, Size),
    ClosePane,
    Input(Vec<u8>),
    Paste(String),
    ResizePane(Size),
    Local(Effect),
}

impl Call {
    fn suspends(&self) -> bool {
        matches!(
            self,
            Call::Local(
                Effect::EditText { .. } | Effect::RunExternal { .. } | Effect::OpenInEditor { .. }
            )
        )
    }
}

fn focus_report(focused: bool) -> Vec<u8> {
    match focused {
        true => b"\x1b[I".to_vec(),
        false => b"\x1b[O".to_vec(),
    }
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
    pub anchor: (i64, u16),
    pub cursor: (i64, u16),
}

impl Selection {
    pub fn ordered(&self) -> ((i64, u16), (i64, u16)) {
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
    Z,
}

pub(crate) enum Pending {
    ReportFailure,
    Repos,
    FormSettings(PathBuf),
    ReviewSettings(SessionId),
    Resume(SessionId),
    Create,
    Trust(Box<Retry>),
    Draft(SessionId, LandingMode),
    Land,
    DiscardPreview(SessionId),
    Usage,
    Reconcile,
    LeftoverPreview {
        repo: PathBuf,
        leftover: LeftoverView,
    },
    Fix(Fix),
}

pub(crate) struct Retry {
    request: Request,
    pending: Pending,
}

pub(crate) struct TrustPrompt {
    pub repo: PathBuf,
    pub hash: String,
    pub items: Vec<String>,
    retry: Box<Retry>,
}

impl TrustPrompt {
    pub fn skipping_teardown(&self) -> Option<Request> {
        match &self.retry.request {
            Request::Discard {
                session,
                skip_teardown: false,
            } => Some(Request::Discard {
                session: session.clone(),
                skip_teardown: true,
            }),
            Request::Land {
                session,
                landing,
                skip_teardown: false,
            } => Some(Request::Land {
                session: session.clone(),
                landing: landing.clone(),
                skip_teardown: true,
            }),
            _ => None,
        }
    }
}

pub(crate) enum Popup {
    New(NewForm),
    Trust(TrustPrompt),
    Land(LandForm),
    Discard(DiscardConfirm),
    Usage(UsageReport),
    Retarget(RetargetPicker),
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
    pub reconcile: Option<ReconcileView>,
    pub reconcile_report: ReconcileReport,
    pub mismatch: Option<String>,
    pub gesture: Option<Region>,
    pub pane_selection: Selector,
    pub links: Hyperlinks,
    pub sidebar: SidebarView,
    pub clock: Box<dyn Fn() -> Instant>,
    size: Size,
    guards: Guards,
    select_when_listed: Option<SessionId>,
    insert_when_open: Option<SessionId>,
    terminal_focused: bool,
    reported_view: Option<(Option<SessionId>, bool)>,
    next_request: u64,
    next_pane: u64,
    external_review_command: Option<String>,
    pending: HashMap<RequestId, (Request, Pending)>,
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
            reconcile: None,
            reconcile_report: ReconcileReport::default(),
            mismatch: None,
            gesture: None,
            pane_selection: Selector::default(),
            links: Hyperlinks::default(),
            sidebar: SidebarView::default(),
            clock: Box::new(Instant::now),
            size,
            guards: Guards::default(),
            select_when_listed: None,
            insert_when_open: None,
            terminal_focused: false,
            reported_view: None,
            next_request: 0,
            next_pane: 0,
            external_review_command: None,
            pending: HashMap::new(),
            calls: Vec::new(),
        }
    }

    pub fn update(&mut self, event: Event) -> Vec<Call> {
        match event {
            Event::Daemon(message) => self.daemon(message),
            Event::Pane { pane, message } => self.pane_message(pane, message),
            Event::Terminal(event) => self.terminal(event),
            Event::EditorClosed(result) => self.editor_closed(result),
            Event::Review {
                session,
                purpose,
                result,
            } => self.review_loaded(session, purpose, result),
            Event::VersionMismatch { message } => self.mismatch = Some(message),
            Event::Notice(text) => self.message = Some(text),
            Event::Tick => crate::mouse::tick(self),
            Event::Disconnected { reason } => {
                self.message = Some(format!("lost the Daemon: {reason}"));
            }
        }
        self.tell_focus();
        if self.calls.iter().any(Call::suspends) {
            self.end_gesture();
        }
        std::mem::take(&mut self.calls)
    }

    fn daemon(&mut self, message: FromDaemon) {
        match message {
            FromDaemon::Sessions { sessions } => {
                self.mismatch = None;
                self.sessions.replace(sessions);
                self.request(Request::Repos, Pending::Repos);
            }
            FromDaemon::SessionChanged { session } => self.sessions.upsert(*session),
            FromDaemon::SessionRemoved { session } => self.remove_session(&session),
            FromDaemon::Reconciled { report } => self.reconciled(*report, false),
            FromDaemon::Ring {
                session,
                title,
                body,
            } => self.ring(&session, &title, &body),
            FromDaemon::RateLimits {
                five_hour,
                seven_day,
            } => {
                self.rate_limits = RateLimits {
                    five_hour,
                    seven_day,
                }
            }
            FromDaemon::Focus { session } => self.focus_session(session),
            FromDaemon::Clipboard { session, text } => {
                if self.selected.as_ref() == Some(&session) {
                    self.copy(&text);
                }
            }
            FromDaemon::Response { id, result } => {
                if let Some((request, pending)) = self.pending.remove(&RequestId(id)) {
                    self.answered(request, pending, result);
                }
            }
            _ => {}
        }
        self.follow_selection();
    }

    fn answered(
        &mut self,
        request: Request,
        pending: Pending,
        result: Result<Reply, RequestError>,
    ) {
        let retriable = !matches!(
            pending,
            Pending::Trust(_) | Pending::Draft(..) | Pending::FormSettings(_)
        );
        let result = match result {
            Err(RequestError::Untrusted { repo, hash, items }) if retriable => {
                let retry = Box::new(Retry { request, pending });
                self.popup = Some(Popup::Trust(TrustPrompt {
                    repo,
                    hash,
                    items,
                    retry,
                }));
                return;
            }
            result => result,
        };
        match (pending, result) {
            (Pending::Repos, Ok(Reply::Repos { repos })) => self.config.repos = repos,
            (Pending::FormSettings(repo), Ok(Reply::RepoSettings(settings))) => {
                if let Some(Popup::New(form)) = &mut self.popup
                    && form.repo_path() == Some(repo)
                {
                    form.apply(settings);
                }
            }
            (Pending::FormSettings(_), _) => {}
            (Pending::ReviewSettings(session), result) => {
                let command = match result {
                    Ok(Reply::RepoSettings(settings)) => settings.review_command,
                    Err(err) => {
                        self.message = Some(format!("using the default review command: {err}"));
                        None
                    }
                    Ok(_) => None,
                };
                self.external_review_command =
                    Some(command.unwrap_or_else(|| self.config.review_command.clone()));
                self.start_review(&session, ReviewPurpose::External);
            }
            (Pending::Create, Ok(Reply::Created { session })) => {
                self.select_when_listed = Some(session);
            }
            (Pending::Trust(retry), Ok(_)) => self.request(retry.request, retry.pending),
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
                let question = match self.sessions.get(&session) {
                    Some(view) => format!("Discard {} / {}?", repo_name(&view.repo), view.slug),
                    None => format!("Discard {}?", self.sessions.slug_or_id(&session)),
                };
                let target = DiscardTarget::Session(session);
                let confirm = DiscardConfirm::new(target, question, uncommitted, unlanded);
                self.popup = Some(Popup::Discard(confirm));
            }
            (
                Pending::LeftoverPreview { repo, leftover },
                Ok(Reply::DiscardPreview {
                    uncommitted,
                    unlanded,
                }),
            ) => {
                let question = format!("Remove the Leftover {}?", leftover.label());
                let target = DiscardTarget::Leftover { repo, leftover };
                let confirm = DiscardConfirm::new(target, question, uncommitted, unlanded);
                self.popup = Some(Popup::Discard(confirm));
            }
            (Pending::Usage, Ok(Reply::Usage(report))) => self.popup = Some(Popup::Usage(report)),
            (Pending::Reconcile, Ok(Reply::Reconciled { report })) => {
                self.reconciled(*report, true)
            }
            (Pending::Fix(fix), Ok(_)) => {
                self.message = Some(format!("Done: {}", fix.label()));
                self.request_reconcile();
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
            FromDaemon::Screen(snapshot) => {
                mirror.restore(&snapshot);
                self.pane_selection.clear();
                self.links.hover(None);
            }
            FromDaemon::Output { bytes } => {
                let grown = mirror.output(&bytes);
                match grown {
                    Some(grown) => self.pane_selection.shift(grown),
                    None => self.pane_selection.clear(),
                }
                match grown.filter(|grown| *grown != 0) {
                    Some(grown) => self.links.shift(grown),
                    None => self.links.hover(None),
                }
            }
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
                self.links.hover(None);
                crate::keymap::handle(self, key)
            }
            TermEvent::Paste(text) => crate::keymap::paste(self, text),
            TermEvent::Mouse(event) => crate::mouse::handle(self, event),
            TermEvent::FocusGained => self.terminal_focus(true),
            TermEvent::FocusLost => self.terminal_focus(false),
            TermEvent::Resize(cols, rows) => self.resize(Size { rows, cols }),
            _ => {}
        }
        self.follow_selection();
    }

    fn terminal_focus(&mut self, focused: bool) {
        self.terminal_focused = focused;
        if !focused {
            self.end_gesture();
        }
    }

    fn tell_focus(&mut self) {
        let focused = self.terminal_focused;
        let wanted = self.shown_pane().is_some_and(PaneMirror::wants_focus);
        let Some(pane) = self.pane.as_mut() else {
            return;
        };
        if !wanted {
            pane.told_focused = false;
        } else if pane.told_focused != focused {
            pane.told_focused = focused;
            self.calls.push(Call::Input(focus_report(focused)));
        }
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

    fn remove_session(&mut self, session: &SessionId) {
        let removed = Stop::Session(session.clone());
        let before = self.sidebar.stops(&self.sessions);
        self.sessions.remove(session);
        if self.cursor() != Some(removed.clone()) {
            return;
        }
        let at = before.iter().position(|stop| *stop == removed).unwrap_or(0);
        let after = self.sidebar.stops(&self.sessions);
        if let Some(next) = after.get(at.min(after.len().saturating_sub(1))) {
            self.set_cursor(next.clone());
        }
    }

    fn focus_session(&mut self, session: SessionId) {
        if self.sessions.get(&session).is_none() {
            return;
        }
        self.set_cursor(Stop::Session(session));
        self.reveal_guards();
        self.popup = None;
        self.review = None;
        self.reconcile = None;
        self.mode = Mode::Normal;
        self.prefix = None;
        self.focus = Focus::Pane;
    }

    fn reconciled(&mut self, report: ReconcileReport, open: bool) {
        self.reconcile_report = report;
        if open || self.reconcile.is_some() {
            let selected = self.reconcile.as_ref().map_or(0, |view| view.selected);
            let view = ReconcileView::new(&self.reconcile_report, &self.sessions, selected);
            self.reconcile = Some(view);
        }
    }

    pub fn findings(&self) -> usize {
        self.reconcile_report.findings().count()
    }

    pub fn request_usage(&mut self) {
        self.request(Request::Usage, Pending::Usage);
    }

    pub fn request_reconcile(&mut self) {
        self.request(Request::Reconcile, Pending::Reconcile);
    }

    pub fn apply_fix(&mut self, fix: Fix) {
        match plan(fix, &self.sessions) {
            FixStep::Preview { repo, leftover } => {
                let request = Request::LeftoverPreview {
                    repo: repo.clone(),
                    leftover: leftover.clone(),
                };
                self.request(request, Pending::LeftoverPreview { repo, leftover });
            }
            FixStep::Pick(picker) => self.popup = Some(Popup::Retarget(picker)),
            FixStep::Send(fix) => self.send_fix(fix),
        }
    }

    pub fn send_fix(&mut self, fix: Fix) {
        let request = Request::Fix { fix: fix.clone() };
        self.request(request, Pending::Fix(fix));
    }

    pub fn areas(&self) -> Areas {
        Areas::for_size(self.size)
    }

    pub fn pane_size(&self) -> Size {
        self.areas().pane_inner()
    }

    pub fn shown_pane(&self) -> Option<&PaneMirror> {
        self.pane
            .as_ref()
            .filter(|pane| self.selected.as_ref() == Some(&pane.session) && pane.closed.is_none())
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
        let stops = self.sidebar.stops(&self.sessions);
        let Some(at) = self
            .cursor()
            .and_then(|cursor| stops.iter().position(|stop| *stop == cursor))
        else {
            return;
        };
        let next = at
            .saturating_add_signed(offset)
            .min(stops.len().saturating_sub(1));
        if next != at {
            self.set_cursor(stops[next].clone());
            self.reveal_guards();
        }
        self.follow_selection();
    }

    pub fn show_session(&mut self, session: SessionId) {
        if self.selected.as_ref() != Some(&session) {
            self.set_cursor(Stop::Session(session));
            self.reveal_guards();
        }
        self.follow_selection();
    }

    pub fn cursor(&self) -> Option<Stop> {
        match (&self.sidebar.heading, &self.selected) {
            (Some(repo), _) => Some(Stop::Heading(repo.clone())),
            (None, Some(session)) => Some(Stop::Session(session.clone())),
            (None, None) => None,
        }
    }

    fn set_cursor(&mut self, stop: Stop) {
        match stop {
            Stop::Session(session) => {
                self.sidebar.heading = None;
                self.selected = Some(session);
            }
            Stop::Heading(repo) => {
                self.sidebar.heading = Some(repo);
                self.selected = None;
            }
        }
    }

    pub fn sidebar_rows(&self) -> Vec<Row<'_>> {
        let width = usize::from(self.areas().sidebar.width.saturating_sub(2));
        self.sidebar
            .rows(&self.sessions, &self.reconcile_report, width)
    }

    pub fn sidebar_viewport(&self, rows: usize) -> Viewport {
        let height = usize::from(self.areas().sidebar.height.saturating_sub(2));
        Viewport { rows, height }
    }

    pub fn sidebar_stop_at(&self, row: u16) -> Option<Stop> {
        let first = self.areas().sidebar.y + 1;
        let rows = self.sidebar_rows();
        let offset = self.sidebar.offset(self.sidebar_viewport(rows.len()));
        let at = usize::from(row.checked_sub(first)?) + offset;
        rows.get(at).and_then(Row::stop)
    }

    pub fn scroll_sidebar(&mut self, lines: isize) {
        let viewport = self.sidebar_viewport(self.sidebar_rows().len());
        self.sidebar.scroll_by(lines, viewport);
    }

    pub fn toggle_fold(&mut self, repo: &std::path::Path) {
        if !self.sidebar.is_folded(repo) {
            self.sidebar.fold(repo);
            if self.selected_view().is_some_and(|view| view.repo == repo) {
                self.set_cursor(Stop::Heading(repo.to_path_buf()));
            }
            return;
        }
        self.sidebar.unfold(repo);
        if self.sidebar.heading.as_deref() != Some(repo) {
            return;
        }
        let first = self
            .sessions
            .in_repo(repo)
            .next()
            .map(|view| view.id.clone());
        if let Some(first) = first {
            self.set_cursor(Stop::Session(first));
            self.reveal_guards();
        }
    }

    pub fn on_heading(&self) -> bool {
        self.sidebar.heading.is_some()
    }

    pub fn toggle_cursor_fold(&mut self) {
        let repo = match self.cursor() {
            Some(Stop::Heading(repo)) => repo,
            Some(Stop::Session(_)) => match self.selected_view() {
                Some(view) => view.repo.clone(),
                None => return,
            },
            None => return,
        };
        self.toggle_fold(&repo);
    }

    fn settle_cursor(&mut self) {
        if let Some(repo) = self.selected_view().map(|view| view.repo.clone()) {
            self.sidebar.unfold(&repo);
        }
        let stops = self.sidebar.stops(&self.sessions);
        if self.cursor().is_some_and(|cursor| stops.contains(&cursor)) {
            return;
        }
        match stops.into_iter().next() {
            Some(first) => self.set_cursor(first),
            None => {
                self.selected = None;
                self.sidebar.heading = None;
            }
        }
    }

    fn reveal_cursor(&mut self) {
        let cursor = self.cursor();
        if cursor == self.sidebar.revealed {
            return;
        }
        self.sidebar.revealed = cursor.clone();
        let Some(cursor) = cursor else {
            return;
        };
        let rows = self.sidebar_rows();
        let viewport = self.sidebar_viewport(rows.len());
        let row = rows
            .iter()
            .position(|row| row.stop() == Some(cursor.clone()));
        drop(rows);
        if let Some(row) = row {
            self.sidebar.reveal(row, viewport);
        }
    }

    pub fn end_gesture(&mut self) {
        self.gesture = None;
        self.pane_selection.let_go();
        if let Some(review) = &mut self.review {
            review.selection.let_go();
        }
    }

    pub fn push(&mut self, call: Call) {
        self.calls.push(call);
    }

    pub fn copy(&mut self, text: &str) {
        for effect in copy_effects(&self.config.display, text) {
            self.push(Call::Local(effect));
        }
    }

    pub fn open_url(&mut self, url: String) {
        self.push(Call::Local(Effect::OpenUrl { url }));
    }

    pub fn open_in_editor(&mut self, cwd: PathBuf, file: PathBuf, line: Option<u32>) {
        let effect = Effect::OpenInEditor { file, line, cwd };
        self.push(Call::Local(effect));
    }

    pub fn edit(&mut self, target: EditorTarget) {
        let worktree = match &self.review {
            Some(review) => Some(review.worktree.clone()),
            None => self.selected_view().map(|view| view.worktree.clone()),
        };
        if let Some(cwd) = worktree {
            self.open_in_editor(cwd, target.file, target.line);
        }
    }

    pub fn review_action(&mut self, action: ReviewAction) {
        match action {
            ReviewAction::Stay => {}
            ReviewAction::Close => self.review = None,
            ReviewAction::CommandLine => self.mode = Mode::CommandLine(String::new()),
            ReviewAction::Open(target) => self.edit(target),
            ReviewAction::Notice(text) => self.message = Some(text),
        }
    }

    pub fn report(&mut self, request: Request) {
        self.request(request, Pending::ReportFailure);
    }

    fn request(&mut self, request: Request, pending: Pending) {
        self.next_request += 1;
        let id = RequestId(self.next_request);
        self.pending.insert(id, (request.clone(), pending));
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
        self.request(Request::CreateSession(create), Pending::Create);
    }

    pub fn approve_trust(&mut self, prompt: TrustPrompt) {
        let request = Request::ApproveTrust {
            repo: prompt.repo,
            hash: prompt.hash,
        };
        self.request(request, Pending::Trust(prompt.retry));
    }

    pub fn skip_teardown(&mut self, prompt: TrustPrompt) {
        if let Some(request) = prompt.skipping_teardown() {
            self.request(request, prompt.retry.pending);
        }
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
        self.form_repo_changed();
    }

    pub fn form_repo_changed(&mut self) {
        self.refresh_base_candidates();
        let Some(Popup::New(form)) = &self.popup else {
            return;
        };
        if let Some(repo) = form.repo_path().filter(|_| !form.other_selected()) {
            let request = Request::RepoSettings { repo: repo.clone() };
            self.request(request, Pending::FormSettings(repo));
        }
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
        self.request(
            Request::Land {
                session,
                landing,
                skip_teardown: false,
            },
            Pending::Land,
        );
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
        let session = view.id.clone();
        match purpose {
            ReviewPurpose::BuiltIn => self.start_review(&session, purpose),
            ReviewPurpose::External => {
                let request = Request::RepoSettings {
                    repo: view.repo.clone(),
                };
                self.request(request, Pending::ReviewSettings(session));
            }
        }
    }

    fn start_review(&mut self, session: &SessionId, purpose: ReviewPurpose) {
        let Some(view) = self.sessions.get(session) else {
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
                let (base, worktree) = (view.base.clone(), view.worktree.clone());
                self.review = Some(ReviewView::new(base, worktree, data.files));
            }
            ReviewPurpose::External => {
                let command = self
                    .external_review_command
                    .take()
                    .unwrap_or_else(|| self.config.review_command.clone());
                let effect = Effect::RunExternal {
                    command,
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
            self.set_cursor(Stop::Session(wanted));
        }
        self.settle_cursor();
        self.sync_pane();
        self.report_view();
        self.reveal_cursor();
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
        if let Some(left) = self.pane.take() {
            if left.told_focused {
                self.calls.push(Call::Input(focus_report(false)));
            }
            self.calls.push(Call::ClosePane);
        }
        let Some((session, holder_pid)) = wanted else {
            return;
        };
        self.pane_selection.clear();
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
