use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, Call, Popup};
use crate::discard::{DiscardConfirm, DiscardTarget};
use crate::event::Effect;
use crate::guard::{GuardAction, GuardId, action};
use crate::land::LandAction;
use crate::new_form::Outcome;
use crate::reconcile::PickAction;

pub(crate) fn guard_key(app: &mut App, guard: GuardId, key: KeyEvent) {
    match action(key) {
        GuardAction::Answer(choice) => app.answer_guard(guard, choice),
        GuardAction::Hide => app.hide_guard(guard),
        GuardAction::Ignore => {}
    }
}

pub(crate) fn key(app: &mut App, key: KeyEvent) {
    let Some(popup) = app.popup.as_mut() else {
        return;
    };
    match popup {
        Popup::New(form) => {
            let outcome = form.key(key);
            new_form(app, outcome);
        }
        Popup::Land(form) => {
            let session = form.session.clone();
            match form.key(key) {
                LandAction::Stay => {}
                LandAction::Cancel => app.popup = None,
                LandAction::Land(landing) => {
                    app.popup = None;
                    app.land(session, landing);
                }
                LandAction::Redraft(mode) => app.request_draft(session, mode),
                LandAction::Edit(text) => app.push(Call::Local(Effect::EditText { text })),
            }
        }
        Popup::Trust(prompt) => match key.code {
            KeyCode::Char('y') => {
                if let Some(Popup::Trust(prompt)) = app.popup.take() {
                    app.approve_trust(prompt);
                }
            }
            KeyCode::Char('s') if prompt.skipping_teardown().is_some() => {
                if let Some(Popup::Trust(prompt)) = app.popup.take() {
                    app.skip_teardown(prompt);
                }
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                if let Some(Popup::Trust(prompt)) = app.popup.take() {
                    app.decline_trust(prompt);
                }
            }
            _ => {}
        },
        Popup::Discard(_) => {
            if let Some(Popup::Discard(confirm)) = app.popup.take()
                && DiscardConfirm::confirms(key)
            {
                match confirm.target {
                    DiscardTarget::Session(session) => {
                        app.report(orch_protocol::Request::Discard {
                            session,
                            skip_teardown: false,
                        })
                    }
                    DiscardTarget::Leftover { repo, leftover } => {
                        app.send_fix(orch_protocol::Fix::RemoveLeftover { repo, leftover })
                    }
                }
            }
        }
        Popup::Usage(_) => {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter) {
                app.popup = None;
            }
        }
        Popup::Retarget(picker) => match picker.key(key) {
            PickAction::Stay => {}
            PickAction::Cancel => app.popup = None,
            PickAction::Pick(fix) => {
                app.popup = None;
                app.send_fix(fix);
            }
        },
    }
}

pub(crate) fn paste(app: &mut App, text: &str) {
    if let Some(Popup::New(form)) = app.popup.as_mut() {
        let outcome = form.paste(text);
        new_form(app, outcome);
    }
}

fn new_form(app: &mut App, outcome: Outcome) {
    match outcome {
        Outcome::Stay => {}
        Outcome::Cancel => app.popup = None,
        Outcome::Submit(create) => {
            app.popup = None;
            app.create_session(create);
        }
        Outcome::Edit(text) => app.push(Call::Local(Effect::EditText { text })),
        Outcome::RepoChanged => app.form_repo_changed(),
    }
}
