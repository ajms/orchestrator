use std::path::Path;

use orch_core::SessionId;
use orch_protocol::{AgentStateView, PhaseView, SessionView};

/// Parent links are followed at most this far, so a cycle can't hang.
const MAX_DEPTH_SHOWN: usize = 16;

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

    /// The Repo's Sessions in tree order: each Session followed by its
    /// Child Sessions.
    pub fn in_repo<'a>(&'a self, repo: &'a Path) -> impl Iterator<Item = &'a SessionView> {
        let mut ordered = Vec::new();
        let roots = self
            .listed()
            .filter(move |view| view.repo == repo)
            .filter(|view| self.listed_parent(view).is_none());
        for root in roots {
            self.push_tree(root, &mut ordered, 0);
        }
        ordered.into_iter()
    }

    fn push_tree<'a>(
        &'a self,
        view: &'a SessionView,
        into: &mut Vec<&'a SessionView>,
        depth: usize,
    ) {
        if into.iter().any(|known| known.id == view.id) || depth > MAX_DEPTH_SHOWN {
            return;
        }
        into.push(view);
        for child in self.children(&view.id) {
            self.push_tree(child, into, depth + 1);
        }
    }

    /// The listed Child Sessions of a Session.
    pub fn children<'a>(&'a self, id: &'a SessionId) -> impl Iterator<Item = &'a SessionView> {
        self.listed()
            .filter(move |view| view.parent.as_ref() == Some(id))
    }

    pub fn has_children(&self, id: &SessionId) -> bool {
        self.children(id).next().is_some()
    }

    /// The listed Parent Session, if the Session is a Child Session.
    pub fn listed_parent(&self, view: &SessionView) -> Option<&SessionView> {
        let parent = view.parent.as_ref()?;
        self.listed().find(|known| &known.id == parent)
    }

    /// The listed ancestors, nearest first.
    pub fn ancestors(&self, view: &SessionView) -> Vec<&SessionView> {
        let mut found: Vec<&SessionView> = Vec::new();
        let mut at = self.listed_parent(view);
        while let Some(parent) = at {
            if found.iter().any(|known| known.id == parent.id) || found.len() > MAX_DEPTH_SHOWN {
                break;
            }
            found.push(parent);
            at = self.listed_parent(parent);
        }
        found
    }

    /// Every listed descendant of a Session.
    pub fn descendants(&self, id: &SessionId) -> Vec<&SessionView> {
        let Some(view) = self.get(id) else {
            return Vec::new();
        };
        let mut tree = Vec::new();
        self.push_tree(view, &mut tree, 0);
        tree.remove(0);
        tree
    }

    pub fn ordered(&self) -> Vec<&SessionView> {
        self.repos()
            .into_iter()
            .flat_map(|repo| self.in_repo(repo).collect::<Vec<_>>())
            .collect()
    }

    pub fn count(&self, state: AgentStateView) -> usize {
        self.listed().filter(|view| agent_is(view, state)).count()
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

pub fn agent_is(view: &SessionView, state: AgentStateView) -> bool {
    view.phase.is_live() && view.agent_state == Some(state)
}

pub fn needs_input(view: &SessionView) -> bool {
    agent_is(view, AgentStateView::NeedsInput)
}

pub fn agent_ended(view: &SessionView) -> bool {
    matches!(
        view.agent_state,
        Some(AgentStateView::Exited | AgentStateView::Errored)
    )
}

pub fn agent_running(view: &SessionView) -> bool {
    view.phase.is_live() && !agent_ended(view)
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
