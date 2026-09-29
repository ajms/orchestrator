use crossterm::event::{KeyCode, KeyEvent};
use orch_core::SessionId;
use std::path::PathBuf;

use orch_protocol::{Finding, Fix, LeftoverView, ReconcileReport};

use crate::sessions::{Sessions, repo_label};

pub(crate) enum Row {
    Heading(String),
    Finding(String),
    Fix(Fix),
}

pub(crate) enum FixStep {
    Preview {
        repo: PathBuf,
        leftover: LeftoverView,
    },
    Pick(RetargetPicker),
    Send(Fix),
}

pub(crate) fn plan(fix: Fix, sessions: &Sessions) -> FixStep {
    match fix {
        Fix::RemoveLeftover { repo, leftover } => FixStep::Preview { repo, leftover },
        Fix::Retarget { session, base } => {
            FixStep::Pick(RetargetPicker::new(sessions, session, base))
        }
        fix => FixStep::Send(fix),
    }
}

pub(crate) enum ReconcileAction {
    Stay,
    Close,
    CommandLine,
    Apply(Fix),
}

pub(crate) struct ReconcileView {
    pub rows: Vec<Row>,
    pub selected: usize,
}

impl ReconcileView {
    pub fn new(report: &ReconcileReport, sessions: &Sessions, selected: usize) -> Self {
        let slug = |id: &SessionId| sessions.slug_or_id(id);
        let mut rows = Vec::new();
        for repo in &report.repos {
            rows.push(Row::Heading(repo_label(&repo.repo, repo.missing)));
            push_findings(&mut rows, &repo.findings, &slug);
        }
        if !report.unknown_holders.is_empty() {
            rows.push(Row::Heading("Unknown Holders".into()));
            push_findings(&mut rows, &report.unknown_holders, &slug);
        }
        let mut view = Self { rows, selected: 0 };
        view.selected = selected.min(view.fixes().count().saturating_sub(1));
        view
    }

    pub fn fixes(&self) -> impl Iterator<Item = &Fix> {
        self.rows.iter().filter_map(|row| match row {
            Row::Fix(fix) => Some(fix),
            _ => None,
        })
    }

    pub fn key(&mut self, key: KeyEvent) -> ReconcileAction {
        let last = self.fixes().count().saturating_sub(1);
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return ReconcileAction::Close,
            KeyCode::Char(':') => return ReconcileAction::CommandLine,
            KeyCode::Char('j') | KeyCode::Down => self.selected = (self.selected + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Enter => {
                if let Some(fix) = self.fixes().nth(self.selected) {
                    return ReconcileAction::Apply(fix.clone());
                }
            }
            _ => {}
        }
        ReconcileAction::Stay
    }
}

fn push_findings(rows: &mut Vec<Row>, findings: &[Finding], slug: &impl Fn(&SessionId) -> String) {
    for finding in findings {
        rows.push(Row::Finding(finding.problem.describe(slug)));
        rows.extend(finding.fixes.iter().cloned().map(Row::Fix));
    }
}

pub(crate) struct RetargetPicker {
    pub session: SessionId,
    pub slug: String,
    pub candidates: Vec<String>,
    pub choice: usize,
}

impl RetargetPicker {
    pub fn new(sessions: &Sessions, session: SessionId, suggested: String) -> Self {
        let mut candidates = vec![suggested];
        if let Some(own) = sessions.get(&session) {
            for view in sessions.in_repo(&own.repo) {
                if view.id != session && !candidates.contains(&view.branch) {
                    candidates.push(view.branch.clone());
                }
            }
        }
        Self {
            slug: sessions.slug_or_id(&session),
            session,
            candidates,
            choice: 0,
        }
    }
}

pub(crate) enum PickAction {
    Stay,
    Cancel,
    Pick(Fix),
}

impl RetargetPicker {
    pub fn key(&mut self, key: KeyEvent) -> PickAction {
        let len = self.candidates.len().max(1);
        match key.code {
            KeyCode::Esc => return PickAction::Cancel,
            KeyCode::Right | KeyCode::Tab => self.choice = (self.choice + 1) % len,
            KeyCode::Left | KeyCode::BackTab => self.choice = (self.choice + len - 1) % len,
            KeyCode::Enter => {
                if let Some(base) = self.candidates.get(self.choice) {
                    return PickAction::Pick(Fix::Retarget {
                        session: self.session.clone(),
                        base: base.clone(),
                    });
                }
            }
            _ => {}
        }
        PickAction::Stay
    }
}
