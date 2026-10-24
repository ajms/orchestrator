use std::collections::HashMap;

use orch_core::SessionId;

use crate::event::PaneId;
use crate::pane::PaneMirror;

pub(crate) struct Shell {
    pub mirror: PaneMirror,
    pub command: Option<String>,
    pub shown: bool,
    pub exited: Option<String>,
}

impl Shell {
    pub fn new(mirror: PaneMirror, command: Option<String>) -> Self {
        Self {
            mirror,
            command,
            shown: true,
            exited: None,
        }
    }

    pub fn running(&self) -> bool {
        self.exited.is_none()
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

    pub fn shown_mut(&mut self, session: Option<&SessionId>) -> Option<&mut Shell> {
        session
            .and_then(|session| self.0.get_mut(session))
            .filter(|shell| shell.shown)
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

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Shell> {
        self.0.values_mut()
    }
}

pub(crate) fn shown_mirror<'a>(
    pane: &'a mut Option<PaneMirror>,
    shells: &'a mut Shells,
    selected: Option<&SessionId>,
) -> Option<&'a mut PaneMirror> {
    match shells.shown_mut(selected) {
        Some(shell) => Some(&mut shell.mirror),
        None => pane.as_mut(),
    }
}
