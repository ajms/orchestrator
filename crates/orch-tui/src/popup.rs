use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, Call, Popup};
use crate::discard::DiscardConfirm;
use crate::event::Effect;
use crate::guard::{GuardAction, GuardId, action};
use crate::land::LandAction;
use crate::new_form::Outcome;

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
        Popup::Trust(_) => match key.code {
            KeyCode::Char('y') => {
                if let Some(Popup::Trust(prompt)) = app.popup.take() {
                    app.approve_trust(prompt);
                }
            }
            KeyCode::Char('n') | KeyCode::Esc => app.popup = None,
            _ => {}
        },
        Popup::Discard(confirm) => {
            let session = confirm.session.clone();
            app.popup = None;
            if DiscardConfirm::confirms(key) {
                app.report(orch_protocol::Request::Discard { session });
            }
        }
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
        Outcome::RepoChanged => app.refresh_base_candidates(),
    }
}
