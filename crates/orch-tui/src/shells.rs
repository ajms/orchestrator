use std::collections::HashMap;

use orch_core::SessionId;

use crate::event::{PaneId, ShellExit};
use crate::pane::PaneMirror;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ShellKind {
    Interactive,
    Command(String),
}

impl ShellKind {
    pub fn command(&self) -> Option<&str> {
        match self {
            Self::Interactive => None,
            Self::Command(command) => Some(command),
        }
    }
}

pub(crate) struct Shell {
    pub mirror: PaneMirror,
    pub kind: ShellKind,
    pub shown: bool,
    pub exit: Option<ShellExit>,
}

impl Shell {
    pub fn new(mirror: PaneMirror, kind: ShellKind) -> Self {
        Self {
            mirror,
            kind,
            shown: true,
            exit: None,
        }
    }

    pub fn running(&self) -> bool {
        self.exit.is_none()
    }
}

#[derive(Default)]
pub(crate) struct Shells(HashMap<SessionId, Shell>);

impl Shells {
    pub fn get(&self, session: &SessionId) -> Option<&Shell> {
        self.0.get(session)
    }

    pub fn get_mut(&mut self, session: &SessionId) -> Option<&mut Shell> {
        self.0.get_mut(session)
    }

    pub fn shown(&self, selected: Option<&SessionId>, on_subagent: bool) -> Option<&Shell> {
        selected
            .and_then(|session| self.0.get(session))
            .filter(|shell| shell.shown && !on_subagent)
    }

    fn shown_mut(&mut self, selected: Option<&SessionId>, on_subagent: bool) -> Option<&mut Shell> {
        selected
            .and_then(|session| self.0.get_mut(session))
            .filter(|shell| shell.shown && !on_subagent)
    }

    pub fn insert(&mut self, session: SessionId, shell: Shell) {
        self.0.insert(session, shell);
    }

    pub fn remove(&mut self, session: &SessionId) -> Option<Shell> {
        self.0.remove(session)
    }

    pub fn session_of(&self, pane: PaneId) -> Option<SessionId> {
        self.0
            .iter()
            .find(|(_, shell)| shell.mirror.id == pane)
            .map(|(session, _)| session.clone())
    }

    pub fn sessions(&self) -> Vec<SessionId> {
        self.0.keys().cloned().collect()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Shell> {
        self.0.values_mut()
    }
}

pub(crate) fn shown_mirror<'a>(
    pane: &'a mut Option<PaneMirror>,
    shells: &'a mut Shells,
    selected: Option<&SessionId>,
    on_subagent: bool,
) -> Option<&'a mut PaneMirror> {
    match shells.shown_mut(selected, on_subagent) {
        Some(shell) => Some(&mut shell.mirror),
        None => pane.as_mut(),
    }
}
