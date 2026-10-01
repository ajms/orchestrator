use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};

use crate::text_input::TextInput;

pub enum PickerOutcome {
    Stay,
    Close,
    Pick(PathBuf),
}

pub struct RepoPicker {
    pub query: TextInput,
    pub choice: usize,
}

impl RepoPicker {
    pub fn new(query: String) -> Self {
        Self {
            query: TextInput::new(query),
            choice: 0,
        }
    }

    pub fn matches<'a>(&self, repos: &'a [PathBuf]) -> Vec<&'a Path> {
        let query = self.query.text().trim();
        repos
            .iter()
            .filter(|repo| repo.to_string_lossy().contains(query))
            .map(PathBuf::as_path)
            .collect()
    }

    pub fn key(&mut self, key: KeyEvent, repos: &[PathBuf]) -> PickerOutcome {
        let matches = self.matches(repos);
        match key.code {
            KeyCode::Esc => return PickerOutcome::Close,
            KeyCode::Enter => return self.pick(&matches),
            KeyCode::Up => self.choice = self.choice.saturating_sub(1),
            KeyCode::Down => {
                self.choice = (self.choice + 1).min(matches.len().saturating_sub(1));
            }
            _ => {
                let before = self.query.text().to_string();
                self.query.key(key);
                if self.query.text() != before {
                    self.choice = 0;
                }
            }
        }
        PickerOutcome::Stay
    }

    pub fn paste(&mut self, text: &str) {
        self.query.insert(text);
        self.choice = 0;
    }

    fn pick(&self, matches: &[&Path]) -> PickerOutcome {
        let typed = self.query.text().trim();
        match matches.get(self.choice) {
            Some(repo) => PickerOutcome::Pick(repo.to_path_buf()),
            None if !typed.is_empty() => PickerOutcome::Pick(PathBuf::from(typed)),
            None => PickerOutcome::Close,
        }
    }
}
