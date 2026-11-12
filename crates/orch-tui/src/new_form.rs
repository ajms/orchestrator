use std::cell::Cell;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use orch_git::slugify;
use orch_protocol::{CreateSession, OpenPr, RepoSettings};

use crate::branch_check::{BranchCheck, LookupKey};
use crate::repo_picker::{KnownRepo, RepoChoice};
use crate::text_input::{Suggested, TextInput};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Repo,
    Prompt,
    Branch,
    Base,
    Preset,
    Agent,
}

const FIELDS: [Field; 6] = [
    Field::Prompt,
    Field::Repo,
    Field::Branch,
    Field::Base,
    Field::Preset,
    Field::Agent,
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
    pub repo: RepoChoice,
    pub prompt: TextInput,
    pub prompt_top: Cell<usize>,
    pub branch: Suggested,
    branch_prefix: String,
    pub base: Suggested,
    pub default_base: Option<String>,
    pub default_preset: Option<String>,
    pub default_agent: Option<String>,
    pub agents: Vec<String>,
    pub agent: Option<String>,
    pub base_candidates: Vec<String>,
    base_choice: usize,
    pub presets: Vec<String>,
    pub preset: Option<usize>,
    wanted_preset: Option<String>,
    pub error: Option<String>,
    pub discarding: bool,
    pub(crate) check: BranchCheck,
    /// Starting a Child Session of this Session, on its Branch.
    pub parent: Option<ParentChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentChoice {
    pub id: orch_core::SessionId,
    pub name: String,
    pub branch: String,
}

impl NewForm {
    pub fn new(
        repos: Vec<KnownRepo>,
        repo: Option<PathBuf>,
        presets: Vec<String>,
        prefix: &str,
    ) -> Self {
        let mut form = Self {
            field: Field::Prompt,
            repo: RepoChoice::new(repos, repo),
            prompt: TextInput::default(),
            prompt_top: Cell::new(0),
            branch: Suggested::suggestion(String::new()),
            branch_prefix: prefix.to_string(),
            base: Suggested::default(),
            default_base: None,
            default_preset: None,
            default_agent: None,
            agents: Vec::new(),
            agent: None,
            base_candidates: Vec::new(),
            base_choice: 0,
            presets,
            preset: None,
            wanted_preset: None,
            error: None,
            discarding: false,
            check: BranchCheck::default(),
            parent: None,
        };
        form.prefill_branch();
        form
    }

    /// Makes this a Child Session form: on the parent's Repo and Branch, with
    /// its Preset and Agent unless others are picked.
    pub fn child_of(&mut self, parent: ParentChoice, preset: &str, agent: &str) {
        self.base.set(parent.branch.clone());
        self.preset = self.presets.iter().position(|known| known == preset);
        self.wanted_preset = Some(preset.to_string());
        self.agent = Some(agent.to_string());
        self.parent = Some(parent);
    }

    pub fn set_prompt(&mut self, prompt: String) {
        self.prompt.set(prompt);
        self.prefill_branch();
    }

    pub fn restore(&mut self, create: CreateSession) {
        self.repo.set(create.repo);
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
        self.agent = create.agent;
        self.prefill_branch();
    }

    pub fn apply(&mut self, settings: RepoSettings) {
        let wanted = self.wanted_preset.take();
        self.presets = settings.presets;
        self.preset =
            wanted.and_then(|name| self.presets.iter().position(|preset| *preset == name));
        self.default_preset = settings.default_preset;
        self.default_base = settings.default_base;
        self.default_agent = Some(settings.agent);
        self.agents = settings.agents;
        self.agent = self
            .agent
            .take()
            .filter(|agent| self.agents.contains(agent));
        self.branch_prefix = settings.branch_prefix;
        self.prefill_branch();
    }

    pub fn set_base_candidates(&mut self, candidates: Vec<String>) {
        self.base_candidates = candidates;
        self.base_choice = 0;
    }

    pub fn key(&mut self, key: KeyEvent, home: Option<&Path>) -> Outcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && matches!(key.code, KeyCode::Char('s' | 'g')) {
            self.repo.close();
        }
        if self.repo.picker().is_some() {
            if !self.repo.key(key) {
                return Outcome::Stay;
            }
            self.field = Field::Prompt;
            return Outcome::RepoChanged;
        }
        if key.code == KeyCode::Esc && self.check.stop_waiting() {
            return Outcome::Stay;
        }
        let discarding = std::mem::take(&mut self.discarding);
        match key.code {
            KeyCode::Esc if discarding || self.prompt.text().is_empty() => Outcome::Cancel,
            KeyCode::Esc => {
                self.discarding = true;
                Outcome::Stay
            }
            KeyCode::Char('r') if ctrl => self.open_picker(String::new(), home),
            KeyCode::Char('s') if ctrl => self.submit(),
            KeyCode::Char('g') if ctrl => Outcome::Edit(self.prompt.text().to_string()),
            KeyCode::Tab => self.move_field(1),
            KeyCode::BackTab => self.move_field(FIELDS.len() - 1),
            _ => match self.field {
                Field::Prompt => self.prompt_key(key),
                Field::Repo => self.repo_key(key, home),
                Field::Branch => self.branch_key(key),
                Field::Base => self.base_key(key),
                Field::Preset => self.preset_key(key),
                Field::Agent => self.agent_key(key),
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

    fn repo_key(&mut self, key: KeyEvent, home: Option<&Path>) -> Outcome {
        match key.code {
            KeyCode::Enter => self.open_picker(String::new(), home),
            KeyCode::Char(c) if plain(key) => self.open_picker(c.to_string(), home),
            _ => Outcome::Stay,
        }
    }

    fn branch_key(&mut self, key: KeyEvent) -> Outcome {
        match key.code {
            KeyCode::Enter => return self.move_field(1),
            KeyCode::Char(c) if plain(key) => self.branch.replace(c.encode_utf8(&mut [0; 4])),
            _ => {
                self.branch.accept();
                self.branch.key(key);
            }
        }
        Outcome::Stay
    }

    pub fn paste(&mut self, text: &str, home: Option<&Path>) -> Outcome {
        self.discarding = false;
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let line = text.lines().next().unwrap_or_default().trim();
        if self.repo.picker().is_some() {
            self.repo.paste(line);
            return Outcome::Stay;
        }
        match self.field {
            Field::Prompt => {
                self.prompt.insert(&text);
                self.prefill_branch();
            }
            Field::Repo => return self.open_picker(line.to_string(), home),
            Field::Branch => self.branch.replace(line),
            Field::Base if self.locking_pr().is_none() => self.base.replace(line),
            Field::Base | Field::Preset | Field::Agent => {}
        }
        Outcome::Stay
    }

    fn base_key(&mut self, key: KeyEvent) -> Outcome {
        match key.code {
            KeyCode::Enter => return self.move_field(1),
            _ if self.locking_pr().is_some() => {}
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

    fn agent_key(&mut self, key: KeyEvent) -> Outcome {
        let len = self.agents.len();
        let at = self
            .shown_agent()
            .and_then(|name| self.agents.iter().position(|agent| agent == name))
            .unwrap_or(0);
        let next = match key.code {
            KeyCode::Enter => return self.move_field(1),
            _ if len == 0 => return Outcome::Stay,
            KeyCode::Left | KeyCode::Up => (at + len - 1) % len,
            KeyCode::Right | KeyCode::Down => (at + 1) % len,
            _ => return Outcome::Stay,
        };
        self.agent = Some(self.agents[next].clone());
        Outcome::Stay
    }

    pub(crate) fn shown_agent(&self) -> Option<&String> {
        self.agent.as_ref().or(self.default_agent.as_ref())
    }

    fn open_picker(&mut self, query: String, home: Option<&Path>) -> Outcome {
        self.repo.open(query, home);
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

    pub(crate) fn derived_branch(&self) -> String {
        format!("{}{}", self.branch_prefix, slugify(self.prompt.text()))
    }

    pub(crate) fn locking_pr(&self) -> Option<&OpenPr> {
        self.check.open_pr()
    }

    fn effective_base(&self) -> &str {
        match self.locking_pr() {
            Some(pr) => &pr.base,
            None => self.base.text().trim(),
        }
    }

    fn prefill_branch(&mut self) {
        if self.branch.suggested() {
            let derived = self.derived_branch();
            if derived != self.branch.text() {
                self.branch.suggest(derived);
            }
        }
    }

    pub(crate) fn lookup_key(&self) -> Option<LookupKey> {
        let branch = self.branch.text().trim();
        if self.branch.suggested() || branch.is_empty() {
            return None;
        }
        let base = self.base.text().trim();
        Some(LookupKey {
            repo: self.repo.path()?.to_path_buf(),
            branch: branch.to_string(),
            base: (!base.is_empty()).then(|| base.to_string()),
        })
    }

    pub(crate) fn base_shown(&self) -> String {
        match (self.base.text().trim(), &self.default_base) {
            ("", Some(default)) => default.clone(),
            ("", None) => "the Repo default".into(),
            (base, _) => base.to_string(),
        }
    }

    pub(crate) fn submit(&mut self) -> Outcome {
        let Some(repo) = self.repo.path().map(Path::to_path_buf) else {
            self.error = Some("enter the Repo's path".into());
            return Outcome::Stay;
        };
        if self.prompt.text().trim().is_empty() {
            self.error = Some("the prompt is empty".into());
            return Outcome::Stay;
        }
        if self.check.refused() {
            return Outcome::Stay;
        }
        if self.check.looking() {
            self.check.wait();
            return Outcome::Stay;
        }
        if let Some(parent) = &self.parent {
            return Outcome::Submit(CreateSession {
                preset: self.preset.map(|at| self.presets[at].clone()),
                agent: self.agent.clone(),
                parent: Some(parent.id.clone()),
                ..CreateSession::new(repo, self.prompt.text())
            });
        }
        let base = self.effective_base();
        let branch = self.branch.text().trim();
        Outcome::Submit(CreateSession {
            repo,
            prompt: self.prompt.text().to_string(),
            branch: (!self.branch.suggested() && !branch.is_empty()).then(|| branch.to_string()),
            base: (!base.is_empty()).then(|| base.to_string()),
            preset: self.preset.map(|at| self.presets[at].clone()),
            agent: self.agent.clone(),
            parent: None,
        })
    }
}

fn plain(key: KeyEvent) -> bool {
    !key.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}
