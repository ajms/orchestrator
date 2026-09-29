use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent};
use orch_core::SessionId;
use orch_protocol::{GuardChoice, GuardPrompt, SessionView};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct GuardId {
    pub session: SessionId,
    pub guard: u64,
}

pub(crate) enum GuardAction {
    Answer(GuardChoice),
    Hide,
    Ignore,
}

pub(crate) fn action(key: KeyEvent) -> GuardAction {
    match key.code {
        KeyCode::Char('1') => GuardAction::Answer(GuardChoice::AllowOnce),
        KeyCode::Char('2') => GuardAction::Answer(GuardChoice::AllowForSession),
        KeyCode::Char('3') => GuardAction::Answer(GuardChoice::Deny),
        KeyCode::Esc => GuardAction::Hide,
        _ => GuardAction::Ignore,
    }
}

#[derive(Default)]
pub(crate) struct Guards {
    hidden: HashSet<GuardId>,
    answered: HashSet<GuardId>,
}

impl Guards {
    pub fn shown<'a>(&'a self, view: &'a SessionView) -> Option<&'a GuardPrompt> {
        self.open(view)
            .find(|prompt| !self.hidden.contains(&id(view, prompt)))
    }

    pub fn waiting_hidden(&self, view: &SessionView) -> bool {
        self.open(view)
            .any(|prompt| self.hidden.contains(&id(view, prompt)))
    }

    pub fn hide(&mut self, guard: GuardId) {
        self.hidden.insert(guard);
    }

    pub fn answer(&mut self, guard: GuardId) {
        self.answered.insert(guard);
    }

    pub fn reveal(&mut self, session: &SessionId) {
        self.hidden.retain(|guard| &guard.session != session);
    }

    fn open<'a>(&'a self, view: &'a SessionView) -> impl Iterator<Item = &'a GuardPrompt> {
        view.guard_prompts
            .iter()
            .filter(move |prompt| !self.answered.contains(&id(view, prompt)))
    }
}

fn id(view: &SessionView, prompt: &GuardPrompt) -> GuardId {
    GuardId {
        session: view.id.clone(),
        guard: prompt.id,
    }
}
