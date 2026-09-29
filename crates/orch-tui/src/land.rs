use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use orch_core::SessionId;
use orch_protocol::{Landing, LandingMode};

pub(crate) enum LandAction {
    Stay,
    Cancel,
    Land(Landing),
    Redraft(LandingMode),
    Edit(String),
}

pub(crate) struct LandForm {
    pub session: SessionId,
    pub mode: LandingMode,
    pub message: String,
    pub drafting: bool,
    edited: bool,
}

impl LandForm {
    pub fn new(session: SessionId) -> Self {
        Self {
            session,
            mode: LandingMode::Squash,
            message: String::new(),
            drafting: true,
            edited: false,
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> LandAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return LandAction::Cancel,
            KeyCode::Enter => return LandAction::Land(self.landing()),
            KeyCode::Char('g') if ctrl => return LandAction::Edit(self.message.clone()),
            KeyCode::Tab | KeyCode::BackTab => {
                self.mode = match self.mode {
                    LandingMode::Squash => LandingMode::Pr,
                    LandingMode::Pr => LandingMode::Squash,
                };
                if !self.edited {
                    self.drafting = true;
                    return LandAction::Redraft(self.mode);
                }
            }
            KeyCode::Backspace => {
                self.message.pop();
                self.edited = true;
            }
            KeyCode::Char(c) if !ctrl => {
                self.message.push(c);
                self.edited = true;
            }
            _ => {}
        }
        LandAction::Stay
    }

    pub fn drafted(&mut self, mode: LandingMode, title: String, body: &str) {
        if mode != self.mode || self.edited {
            return;
        }
        self.drafting = false;
        self.message = match body.trim() {
            "" => title,
            body => format!("{title}\n\n{body}"),
        };
    }

    pub fn draft_failed(&mut self) {
        self.drafting = false;
    }

    pub fn edited_text(&mut self, text: &str) {
        self.message = text.trim_end().to_string();
        self.edited = true;
    }

    fn landing(&self) -> Landing {
        let message = self.message.trim().to_string();
        match self.mode {
            LandingMode::Squash => Landing::Squash { message },
            LandingMode::Pr => {
                let (title, body) = message.split_once('\n').unwrap_or((&message, ""));
                Landing::Pr {
                    title: title.trim().to_string(),
                    body: body.trim().to_string(),
                }
            }
        }
    }
}
