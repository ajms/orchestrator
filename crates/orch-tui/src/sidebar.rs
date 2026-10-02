use std::collections::HashSet;
use std::path::{Path, PathBuf};

use orch_core::SessionId;
use orch_protocol::{PrChecksView, PrReviewView, ReconcileReport, SessionView, SubagentView};

use crate::preparing::{Preparing, PreparingId};
use crate::sessions::{Sessions, needs_input};

pub(crate) const FLAG_INDENT: usize = 4;
pub(crate) const FLAG_SEPARATOR: &str = " · ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Stop {
    Session(SessionId),
    Heading(PathBuf),
    Preparing(PreparingId),
    Subagent(SessionId, String),
    SubagentsDone(SessionId),
}

impl Stop {
    pub fn session(&self) -> Option<&SessionId> {
        match self {
            Stop::Session(session) | Stop::Subagent(session, _) | Stop::SubagentsDone(session) => {
                Some(session)
            }
            Stop::Heading(_) | Stop::Preparing(_) => None,
        }
    }

    pub fn outer(self) -> Stop {
        match self {
            Stop::Subagent(session, _) | Stop::SubagentsDone(session) => Stop::Session(session),
            stop => stop,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SubagentRow {
    Subagent(String),
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Urgency {
    NeedsInput,
    Unseen,
}

pub(crate) struct Folded {
    pub count: usize,
    pub urgency: Option<Urgency>,
}

pub(crate) enum Row<'a> {
    Blank,
    Heading {
        repo: &'a Path,
        missing: bool,
        folded: Option<Folded>,
    },
    MissingRepo(&'a Path),
    Session(&'a SessionView),
    Flags(&'a SessionView, Vec<Flag>),
    Subagent(&'a SessionView, &'a SubagentView),
    SubagentsDone(&'a SessionView, usize),
    Preparing(&'a Preparing),
}

impl Row<'_> {
    pub fn stop(&self) -> Option<Stop> {
        match self {
            Row::Blank | Row::MissingRepo(_) => None,
            Row::Preparing(preparing) => Some(Stop::Preparing(preparing.id)),
            Row::Heading { repo, .. } => Some(Stop::Heading(repo.to_path_buf())),
            Row::Session(view) | Row::Flags(view, _) => Some(Stop::Session(view.id.clone())),
            Row::Subagent(view, subagent) => {
                Some(Stop::Subagent(view.id.clone(), subagent.id.clone()))
            }
            Row::SubagentsDone(view, _) => Some(Stop::SubagentsDone(view.id.clone())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flag {
    Pr(u64),
    Checks(PrChecksView),
    Review(PrReviewView),
    NewComments(u32),
    PrClosed,
    NeedsRebase,
    Stalled,
    Recovered,
    WorktreeMissing,
    BaseMissing,
    Muted,
}

impl Flag {
    pub fn text(self) -> String {
        match self {
            Flag::Pr(number) => format!("PR #{number}"),
            Flag::Checks(PrChecksView::Pending) => "⋯ checks".into(),
            Flag::Checks(PrChecksView::Passing) => "✓ checks".into(),
            Flag::Checks(_) => "✗ checks".into(),
            Flag::Review(PrReviewView::Approved) => "approved".into(),
            Flag::Review(PrReviewView::ChangesRequested) => "changes requested".into(),
            Flag::Review(_) => "review required".into(),
            Flag::NewComments(count) => format!("{count} new"),
            Flag::PrClosed => "PR closed (:abandon)".into(),
            Flag::NeedsRebase => "⟲ rebase".into(),
            Flag::Stalled => "stalled?".into(),
            Flag::Recovered => "recovered".into(),
            Flag::WorktreeMissing => "worktree missing".into(),
            Flag::BaseMissing => "base missing".into(),
            Flag::Muted => "muted".into(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Viewport {
    pub rows: usize,
    pub height: usize,
}

impl Viewport {
    fn last_offset(self) -> usize {
        self.rows.saturating_sub(self.height)
    }
}

#[derive(Default)]
pub(crate) struct SidebarView {
    folded: HashSet<PathBuf>,
    expanded: HashSet<SessionId>,
    scroll: usize,
    pub heading: Option<PathBuf>,
    pub preparing_row: Option<PreparingId>,
    pub subagent_row: Option<SubagentRow>,
    pub revealed: Option<Stop>,
}

impl SidebarView {
    pub fn is_folded(&self, repo: &Path) -> bool {
        self.folded.contains(repo)
    }

    pub fn fold(&mut self, repo: &Path) {
        self.folded.insert(repo.to_path_buf());
    }

    pub fn unfold(&mut self, repo: &Path) {
        self.folded.remove(repo);
    }

    pub fn is_expanded(&self, session: &SessionId) -> bool {
        self.expanded.contains(session)
    }

    pub fn expand(&mut self, session: &SessionId) {
        self.expanded.insert(session.clone());
    }

    pub fn toggle_expanded(&mut self, session: &SessionId) {
        if !self.expanded.remove(session) {
            self.expand(session);
        }
    }

    pub fn subagent_stops(&self, view: &SessionView) -> Vec<Stop> {
        let mut stops = vec![Stop::Session(view.id.clone())];
        stops.extend(
            subagent_rows(view, self.is_expanded(&view.id))
                .iter()
                .filter_map(Row::stop),
        );
        stops
    }

    pub fn offset(&self, viewport: Viewport) -> usize {
        self.scroll.min(viewport.last_offset())
    }

    pub fn scroll_by(&mut self, lines: isize, viewport: Viewport) {
        self.scroll = self
            .offset(viewport)
            .saturating_add_signed(-lines)
            .min(viewport.last_offset());
    }

    pub fn reveal(&mut self, row: usize, viewport: Viewport) {
        let offset = self.offset(viewport);
        self.scroll = match row {
            row if row < offset => row,
            row if row >= offset + viewport.height => row + 1 - viewport.height,
            _ => offset,
        };
    }

    pub fn stops(&self, sessions: &Sessions, preparing: &[Preparing]) -> Vec<Stop> {
        let mut stops = Vec::new();
        for repo in repos(sessions, preparing) {
            match self.is_folded(repo) {
                true => stops.push(Stop::Heading(repo.to_path_buf())),
                false => {
                    stops.extend(
                        sessions
                            .in_repo(repo)
                            .map(|view| Stop::Session(view.id.clone())),
                    );
                    stops.extend(
                        preparing
                            .iter()
                            .filter(|preparing| preparing.is_in(repo))
                            .map(|preparing| Stop::Preparing(preparing.id)),
                    );
                }
            }
        }
        stops
    }

    pub fn rows<'a>(
        &self,
        sessions: &'a Sessions,
        preparing: &'a [Preparing],
        report: &'a ReconcileReport,
        width: usize,
    ) -> Vec<Row<'a>> {
        let mut rows = Vec::new();
        for repo in repos(sessions, preparing) {
            if !rows.is_empty() {
                rows.push(Row::Blank);
            }
            let views: Vec<&SessionView> = sessions.in_repo(repo).collect();
            let missing = views.iter().any(|view| view.flags.repo_missing);
            let folded = self.is_folded(repo).then(|| Folded {
                count: views.len(),
                urgency: urgency(&views),
            });
            let unfolded = folded.is_none();
            rows.push(Row::Heading {
                repo,
                missing,
                folded,
            });
            if unfolded {
                rows.extend(views.into_iter().flat_map(|view| {
                    let expanded = self.is_expanded(&view.id);
                    session_rows(view, width, expanded)
                }));
                rows.extend(
                    preparing
                        .iter()
                        .filter(|preparing| preparing.is_in(repo))
                        .map(Row::Preparing),
                );
            }
        }
        let listed = sessions.repos();
        for report in &report.repos {
            if report.missing && !listed.contains(&report.repo.as_path()) {
                if !rows.is_empty() {
                    rows.push(Row::Blank);
                }
                rows.push(Row::MissingRepo(&report.repo));
            }
        }
        rows
    }
}

fn repos<'a>(sessions: &'a Sessions, preparing: &'a [Preparing]) -> Vec<&'a Path> {
    let mut repos = sessions.repos();
    for preparing in preparing {
        if !repos.contains(&preparing.create.repo.as_path()) {
            repos.push(&preparing.create.repo);
        }
    }
    repos
}

fn urgency(views: &[&SessionView]) -> Option<Urgency> {
    if views.iter().any(|view| needs_input(view)) {
        return Some(Urgency::NeedsInput);
    }
    views
        .iter()
        .any(|view| view.flags.unseen)
        .then_some(Urgency::Unseen)
}

fn session_rows(view: &SessionView, width: usize, expanded: bool) -> Vec<Row<'_>> {
    let mut rows = vec![Row::Session(view)];
    rows.extend(
        wrap(flags(view), width)
            .into_iter()
            .map(|flags| Row::Flags(view, flags)),
    );
    rows.extend(subagent_rows(view, expanded));
    rows
}

fn subagent_rows(view: &SessionView, expanded: bool) -> Vec<Row<'_>> {
    let mut rows = Vec::new();
    rows.extend(
        view.subagents
            .iter()
            .filter(|subagent| !subagent.done)
            .map(|subagent| Row::Subagent(view, subagent)),
    );
    let done = view
        .subagents
        .iter()
        .filter(|subagent| subagent.done)
        .count();
    if done > 0 {
        rows.push(Row::SubagentsDone(view, done));
    }
    if expanded {
        rows.extend(
            view.subagents
                .iter()
                .filter(|subagent| subagent.done)
                .map(|subagent| Row::Subagent(view, subagent)),
        );
    }
    rows
}

fn flags(view: &SessionView) -> Vec<Flag> {
    let flags = &view.flags;
    let mut found = Vec::new();
    found.extend(flags.pr_number.map(Flag::Pr));
    if let Some(pr) = &flags.pr {
        if pr.checks != PrChecksView::None {
            found.push(Flag::Checks(pr.checks));
        }
        if pr.review != PrReviewView::None {
            found.push(Flag::Review(pr.review));
        }
        if pr.new_comments > 0 {
            found.push(Flag::NewComments(pr.new_comments));
        }
        if pr.closed {
            found.push(Flag::PrClosed);
        }
    }
    let marks = [
        (flags.needs_rebase, Flag::NeedsRebase),
        (flags.stalled, Flag::Stalled),
        (flags.recovered, Flag::Recovered),
        (flags.worktree_missing, Flag::WorktreeMissing),
        (flags.base_missing, Flag::BaseMissing),
        (flags.muted, Flag::Muted),
    ];
    found.extend(
        marks
            .into_iter()
            .filter(|(set, _)| *set)
            .map(|(_, flag)| flag),
    );
    found
}

fn wrap(flags: Vec<Flag>, width: usize) -> Vec<Vec<Flag>> {
    let mut lines = Vec::new();
    let mut current: Vec<Flag> = Vec::new();
    let separator = FLAG_SEPARATOR.chars().count();
    let mut used = 0;
    for flag in flags {
        let len = flag.text().chars().count();
        if !current.is_empty() && used + separator + len > width {
            lines.push(std::mem::take(&mut current));
        }
        used = match current.is_empty() {
            true => FLAG_INDENT,
            false => used + separator,
        } + len;
        current.push(flag);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}
