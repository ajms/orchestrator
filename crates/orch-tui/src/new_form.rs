use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use orch_git::slugify;
use orch_protocol::CreateSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Repo,
    Prompt,
    Branch,
    Base,
    Preset,
}

const FIELDS: [Field; 5] = [
    Field::Repo,
    Field::Prompt,
    Field::Branch,
    Field::Base,
    Field::Preset,
];

pub enum Outcome {
    Stay,
    Cancel,
    Submit(CreateSession),
    Edit(String),
    RepoChanged,
}

pub struct NewForm {
    pub field: Field,
    pub repos: Vec<PathBuf>,
    pub repo: usize,
    pub other_path: String,
    pub prompt: String,
    pub branch: String,
    branch_edited: bool,
    branch_prefix: String,
    pub base: String,
    pub base_candidates: Vec<String>,
    base_choice: usize,
    pub presets: Vec<String>,
    pub preset: Option<usize>,
    pub error: Option<String>,
}

impl NewForm {
    pub fn new(repos: Vec<PathBuf>, preselect: usize, presets: Vec<String>, prefix: &str) -> Self {
        let mut form = Self {
            field: Field::Prompt,
            repos,
            repo: preselect,
            other_path: String::new(),
            prompt: String::new(),
            branch: String::new(),
            branch_edited: false,
            branch_prefix: prefix.to_string(),
            base: String::new(),
            base_candidates: Vec::new(),
            base_choice: 0,
            presets,
            preset: None,
            error: None,
        };
        form.prefill_branch();
        form
    }

    pub fn repo_path(&self) -> Option<PathBuf> {
        match self.repos.get(self.repo) {
            Some(repo) => Some(repo.clone()),
            None => Some(PathBuf::from(self.other_path.trim()))
                .filter(|path| !path.as_os_str().is_empty()),
        }
    }

    pub fn other_selected(&self) -> bool {
        self.repo == self.repos.len()
    }

    pub fn set_prompt(&mut self, prompt: String) {
        self.prompt = prompt;
        self.prefill_branch();
    }

    pub fn set_base_candidates(&mut self, candidates: Vec<String>) {
        self.base_candidates = candidates;
        self.base_choice = 0;
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return Outcome::Cancel,
            KeyCode::Char('s') if ctrl => return self.submit(),
            KeyCode::Char('g') if ctrl && self.field == Field::Prompt => {
                return Outcome::Edit(self.prompt.clone());
            }
            KeyCode::Tab => self.move_field(1),
            KeyCode::BackTab => self.move_field(FIELDS.len() - 1),
            KeyCode::Enter if self.field == Field::Prompt => self.edit_text('\n'),
            KeyCode::Enter => return self.submit(),
            KeyCode::Left => return self.cycle(false),
            KeyCode::Right => return self.cycle(true),
            KeyCode::Backspace => self.erase(),
            KeyCode::Char(c) if !ctrl => self.edit_text(c),
            _ => {}
        }
        Outcome::Stay
    }

    fn move_field(&mut self, step: usize) {
        let at = FIELDS
            .iter()
            .position(|field| *field == self.field)
            .unwrap_or(0);
        self.field = FIELDS[(at + step) % FIELDS.len()];
    }

    fn cycle(&mut self, forward: bool) -> Outcome {
        let step = |at: usize, len: usize| match forward {
            true => (at + 1) % len,
            false => (at + len - 1) % len,
        };
        match self.field {
            Field::Repo => {
                self.repo = step(self.repo, self.repos.len() + 1);
                return Outcome::RepoChanged;
            }
            Field::Base => {
                let len = self.base_candidates.len() + 1;
                self.base_choice = step(self.base_choice, len);
                self.base = match self.base_choice {
                    0 => String::new(),
                    at => self.base_candidates[at - 1].clone(),
                };
            }
            Field::Preset => {
                let len = self.presets.len() + 1;
                let at = self.preset.map_or(0, |at| at + 1);
                self.preset = step(at, len).checked_sub(1);
            }
            Field::Prompt | Field::Branch => {}
        }
        Outcome::Stay
    }

    fn edit_text(&mut self, c: char) {
        match self.field {
            Field::Repo if self.other_selected() => self.other_path.push(c),
            Field::Repo | Field::Preset => {}
            Field::Prompt => {
                self.prompt.push(c);
                self.prefill_branch();
            }
            Field::Branch => {
                self.branch.push(c);
                self.branch_edited = true;
            }
            Field::Base => self.base.push(c),
        }
    }

    fn erase(&mut self) {
        match self.field {
            Field::Repo if self.other_selected() => {
                self.other_path.pop();
            }
            Field::Repo | Field::Preset => {}
            Field::Prompt => {
                self.prompt.pop();
                self.prefill_branch();
            }
            Field::Branch => {
                self.branch.pop();
                self.branch_edited = true;
            }
            Field::Base => {
                self.base.pop();
            }
        }
    }

    fn prefill_branch(&mut self) {
        if !self.branch_edited {
            self.branch = format!("{}{}", self.branch_prefix, slugify(&self.prompt));
        }
    }

    fn submit(&mut self) -> Outcome {
        let Some(repo) = self.repo_path() else {
            self.error = Some("enter the Repo's path".into());
            return Outcome::Stay;
        };
        if self.prompt.trim().is_empty() {
            self.error = Some("the prompt is empty".into());
            return Outcome::Stay;
        }
        let base = self.base.trim();
        let branch = self.branch.trim();
        Outcome::Submit(CreateSession {
            repo,
            prompt: self.prompt.clone(),
            branch: (self.branch_edited && !branch.is_empty()).then(|| branch.to_string()),
            base: (!base.is_empty()).then(|| base.to_string()),
            preset: self.preset.map(|at| self.presets[at].clone()),
        })
    }
}
