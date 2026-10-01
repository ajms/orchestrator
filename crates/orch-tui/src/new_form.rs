use std::cell::Cell;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use orch_git::slugify;
use orch_protocol::{CreateSession, RepoSettings};

use crate::repo_picker::{PickerOutcome, RepoPicker};
use crate::text_input::TextInput;

use crate::text_input::Suggested;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Repo,
    Prompt,
    Branch,
    Base,
    Preset,
}

const FIELDS: [Field; 5] = [
    Field::Prompt,
    Field::Repo,
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
    pub repo: Option<PathBuf>,
    pub picker: Option<RepoPicker>,
    pub prompt: TextInput,
    pub prompt_top: Cell<usize>,
    pub branch: Suggested,
    branch_prefix: String,
    pub base: Suggested,
    pub default_base: Option<String>,
    pub default_preset: Option<String>,
    pub base_candidates: Vec<String>,
    base_choice: usize,
    pub presets: Vec<String>,
    pub preset: Option<usize>,
    wanted_preset: Option<String>,
    pub error: Option<String>,
    pub discarding: bool,
}

impl NewForm {
    pub fn new(
        repos: Vec<PathBuf>,
        repo: Option<PathBuf>,
        presets: Vec<String>,
        prefix: &str,
    ) -> Self {
        let mut form = Self {
            field: Field::Prompt,
            repos,
            repo,
            picker: None,
            prompt: TextInput::default(),
            prompt_top: Cell::new(0),
            branch: Suggested::suggestion(String::new()),
            branch_prefix: prefix.to_string(),
            base: Suggested::default(),
            default_base: None,
            default_preset: None,
            base_candidates: Vec::new(),
            base_choice: 0,
            presets,
            preset: None,
            wanted_preset: None,
            error: None,
            discarding: false,
        };
        form.prefill_branch();
        form
    }

    pub fn repo_path(&self) -> Option<PathBuf> {
        self.repo.clone()
    }

    pub fn set_prompt(&mut self, prompt: String) {
        self.prompt.set(prompt);
        self.prefill_branch();
    }

    pub fn restore(&mut self, create: CreateSession) {
        self.repo = Some(create.repo);
        self.prompt.set(create.prompt);
        if let Some(branch) = create.branch {
            self.branch.set(branch);
        }
        self.base.set(create.base.unwrap_or_default());
        self.preset = create
            .preset
            .as_ref()
            .and_then(|name| self.presets.iter().position(|preset| preset == name));
        self.wanted_preset = create.preset;
        self.prefill_branch();
    }

    pub fn apply(&mut self, settings: RepoSettings) {
        let wanted = self.wanted_preset.take();
        self.presets = settings.presets;
        self.preset =
            wanted.and_then(|name| self.presets.iter().position(|preset| *preset == name));
        self.default_preset = settings.default_preset;
        self.default_base = settings.default_base;
        self.branch_prefix = settings.branch_prefix;
        self.prefill_branch();
    }

    pub fn set_base_candidates(&mut self, candidates: Vec<String>) {
        self.base_candidates = candidates;
        self.base_choice = 0;
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(key.code, KeyCode::Char('s' | 'g')) {
            self.picker = None;
        }
        if let Some(picker) = &mut self.picker {
            return match picker.key(key, &self.repos) {
                PickerOutcome::Stay => Outcome::Stay,
                PickerOutcome::Close => {
                    self.picker = None;
                    Outcome::Stay
                }
                PickerOutcome::Pick(repo) => {
                    self.picker = None;
                    self.repo = Some(repo);
                    Outcome::RepoChanged
                }
            };
        }
        let discarding = std::mem::take(&mut self.discarding);
        match key.code {
            KeyCode::Esc if discarding || self.prompt.text().is_empty() => Outcome::Cancel,
            KeyCode::Esc => {
                self.discarding = true;
                Outcome::Stay
            }
            KeyCode::Char('r') if ctrl => self.open_picker(String::new()),
            KeyCode::Char('s') if ctrl => self.submit(),
            KeyCode::Char('g') if ctrl => Outcome::Edit(self.prompt.text().to_string()),
            KeyCode::Tab => self.move_field(1),
            KeyCode::BackTab => self.move_field(FIELDS.len() - 1),
            _ => match self.field {
                Field::Prompt => self.prompt_key(key),
                Field::Repo => self.repo_key(key),
                Field::Branch => self.branch_key(key),
                Field::Base => self.base_key(key),
                Field::Preset => self.preset_key(key),
            },
        }
    }

    fn prompt_key(&mut self, key: KeyEvent) -> Outcome {
        match key.code {
            KeyCode::Enter => self.prompt.insert("\n"),
            KeyCode::Up => self.prompt.up(),
            KeyCode::Down => self.prompt.down(),
            _ => {
                self.prompt.key(key);
            }
        }
        self.prefill_branch();
        Outcome::Stay
    }

    fn repo_key(&mut self, key: KeyEvent) -> Outcome {
        match key.code {
            KeyCode::Enter => self.open_picker(String::new()),
            KeyCode::Char(c) if plain(key) => self.open_picker(c.to_string()),
            _ => Outcome::Stay,
        }
    }

    fn branch_key(&mut self, key: KeyEvent) -> Outcome {
        if key.code == KeyCode::Enter {
            return self.move_field(1);
        }
        if self.branch.key(key) {
            self.branch_changed();
        }
        Outcome::Stay
    }

    fn branch_changed(&mut self) {
        if self.branch.text().is_empty() {
            self.branch.suggest(String::new());
        }
        self.prefill_branch();
    }

    pub fn paste(&mut self, text: &str) -> Outcome {
        self.discarding = false;
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let line = text.lines().next().unwrap_or_default().trim();
        if let Some(picker) = &mut self.picker {
            picker.paste(line);
            return Outcome::Stay;
        }
        match self.field {
            Field::Prompt => {
                self.prompt.insert(&text);
                self.prefill_branch();
            }
            Field::Repo => return self.open_picker(line.to_string()),
            Field::Branch => {
                self.branch.replace(line);
                self.branch_changed();
            }
            Field::Base => self.base.replace(line),
            Field::Preset => {}
        }
        Outcome::Stay
    }

    fn base_key(&mut self, key: KeyEvent) -> Outcome {
        match key.code {
            KeyCode::Enter => return self.move_field(1),
            KeyCode::Up => self.cycle_base(false),
            KeyCode::Down => self.cycle_base(true),
            KeyCode::Char(c) if plain(key) => self.base.replace(c.encode_utf8(&mut [0; 4])),
            _ => {
                self.base.accept();
                self.base.key(key);
            }
        }
        Outcome::Stay
    }

    fn preset_key(&mut self, key: KeyEvent) -> Outcome {
        let len = self.presets.len() + 1;
        let at = self.preset.map_or(0, |at| at + 1);
        let next = match key.code {
            KeyCode::Enter => return self.move_field(1),
            KeyCode::Left | KeyCode::Up => (at + len - 1) % len,
            KeyCode::Right | KeyCode::Down => (at + 1) % len,
            _ => return Outcome::Stay,
        };
        self.preset = next.checked_sub(1);
        Outcome::Stay
    }

    fn open_picker(&mut self, query: String) -> Outcome {
        self.picker = Some(RepoPicker::new(query));
        Outcome::Stay
    }

    fn move_field(&mut self, step: usize) -> Outcome {
        let at = FIELDS
            .iter()
            .position(|field| *field == self.field)
            .unwrap_or(0);
        self.field = FIELDS[(at + step) % FIELDS.len()];
        Outcome::Stay
    }

    fn cycle_base(&mut self, forward: bool) {
        let len = self.base_candidates.len() + 1;
        self.base_choice = match forward {
            true => (self.base_choice + 1) % len,
            false => (self.base_choice + len - 1) % len,
        };
        self.base.suggest(match self.base_choice {
            0 => String::new(),
            at => self.base_candidates[at - 1].clone(),
        });
    }

    fn prefill_branch(&mut self) {
        if self.branch.suggested() {
            let derived = format!("{}{}", self.branch_prefix, slugify(self.prompt.text()));
            if derived != self.branch.text() {
                self.branch.suggest(derived);
            }
        }
    }

    fn submit(&mut self) -> Outcome {
        let Some(repo) = self.repo_path() else {
            self.error = Some("enter the Repo's path".into());
            return Outcome::Stay;
        };
        if self.prompt.text().trim().is_empty() {
            self.error = Some("the prompt is empty".into());
            return Outcome::Stay;
        }
        let base = self.base.text().trim();
        let branch = self.branch.text().trim();
        Outcome::Submit(CreateSession {
            repo,
            prompt: self.prompt.text().to_string(),
            branch: (!self.branch.suggested() && !branch.is_empty()).then(|| branch.to_string()),
            base: (!base.is_empty()).then(|| base.to_string()),
            preset: self.preset.map(|at| self.presets[at].clone()),
        })
    }
}

fn plain(key: KeyEvent) -> bool {
    !key.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}
