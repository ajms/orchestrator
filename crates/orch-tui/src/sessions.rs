use std::path::Path;

use orch_core::SessionId;
use orch_protocol::{AgentStateView, PhaseView, SessionView};

#[derive(Default)]
pub struct Sessions {
    views: Vec<SessionView>,
}

impl Sessions {
    pub fn replace(&mut self, views: Vec<SessionView>) {
        self.views.clear();
        for view in views {
            self.upsert(view);
        }
    }

    pub fn upsert(&mut self, view: SessionView) {
        match self.views.iter_mut().find(|known| known.id == view.id) {
            Some(known) => *known = view,
            None => self.views.push(view),
        }
    }

    pub fn slug_or_id(&self, id: &SessionId) -> String {
        self.get(id)
            .map_or_else(|| id.as_str().to_string(), |view| view.slug.clone())
    }

    pub fn remove(&mut self, id: &SessionId) {
        self.views.retain(|view| &view.id != id);
    }

    pub fn get(&self, id: &SessionId) -> Option<&SessionView> {
        self.views.iter().find(|view| &view.id == id)
    }

    pub fn repos(&self) -> Vec<&Path> {
        let mut repos: Vec<&Path> = Vec::new();
        for view in self.listed() {
            if !repos.contains(&view.repo.as_path()) {
                repos.push(&view.repo);
            }
        }
        repos
    }

    pub fn in_repo<'a>(&'a self, repo: &'a Path) -> impl Iterator<Item = &'a SessionView> {
        self.listed().filter(move |view| view.repo == repo)
    }

    pub fn ordered(&self) -> Vec<&SessionView> {
        self.repos()
            .into_iter()
            .flat_map(|repo| self.in_repo(repo).collect::<Vec<_>>())
            .collect()
    }

    pub fn count(&self, state: AgentStateView) -> usize {
        self.listed()
            .filter(|view| view.agent == Some(state) && view.phase.is_live())
            .count()
    }

    pub fn unseen(&self) -> usize {
        self.listed().filter(|view| view.flags.unseen).count()
    }

    pub fn ordered_ids(&self) -> Vec<SessionId> {
        self.ordered()
            .into_iter()
            .map(|view| view.id.clone())
            .collect()
    }

    fn listed(&self) -> impl Iterator<Item = &SessionView> {
        self.views
            .iter()
            .filter(|view| !matches!(view.phase, PhaseView::Landed | PhaseView::Discarded))
    }
}

pub fn repo_label(repo: &Path, missing: bool) -> String {
    match missing {
        true => format!("{} (missing)", repo_name(repo)),
        false => repo_name(repo),
    }
}

pub fn repo_name(repo: &Path) -> String {
    repo.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| repo.display().to_string())
}

pub fn phase_label(phase: PhaseView) -> &'static str {
    match phase {
        PhaseView::SettingUp => "Setting up",
        PhaseView::SetupFailed => "Setup failed",
        PhaseView::Active => "Active",
        PhaseView::PrOpen => "PR open",
        PhaseView::Suspended => "Suspended",
        PhaseView::Landed => "Landed",
        PhaseView::Discarded => "Discarded",
    }
}
