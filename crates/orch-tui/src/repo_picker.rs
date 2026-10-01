use std::cell::Cell;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use orch_git::Repo;

use crate::sessions::repo_name;
use crate::text_input::TextInput;

const MAX_FOLDERS: usize = 200;

pub struct RepoChoice {
    known: Vec<KnownRepo>,
    path: Option<PathBuf>,
    moved: bool,
    picker: Option<RepoPicker>,
}

impl RepoChoice {
    pub fn new(known: Vec<KnownRepo>, path: Option<PathBuf>) -> Self {
        Self {
            known,
            path,
            moved: false,
            picker: None,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn moved(&self) -> bool {
        self.moved
    }

    pub fn picker(&self) -> Option<&RepoPicker> {
        self.picker.as_ref()
    }

    pub fn set(&mut self, path: PathBuf) {
        self.path = Some(path);
        self.moved = false;
    }

    pub fn open(&mut self, query: String, home: Option<&Path>) {
        self.picker = Some(RepoPicker::new(
            query,
            self.known.clone(),
            self.path.as_deref(),
            home.map(Path::to_path_buf),
        ));
    }

    pub fn close(&mut self) {
        self.picker = None;
    }

    pub fn key(&mut self, key: KeyEvent) -> bool {
        let Some(picker) = &mut self.picker else {
            return false;
        };
        match picker.key(key) {
            PickerOutcome::Stay => false,
            PickerOutcome::Close => {
                self.picker = None;
                false
            }
            PickerOutcome::Pick { root, moved } => {
                self.picker = None;
                self.path = Some(root);
                self.moved = moved;
                true
            }
        }
    }

    pub fn paste(&mut self, text: &str) {
        if let Some(picker) = &mut self.picker {
            picker.paste(text);
        }
    }
}

enum PickerOutcome {
    Stay,
    Close,
    Pick { root: PathBuf, moved: bool },
}

#[derive(Debug, Clone)]
pub struct KnownRepo {
    pub path: PathBuf,
    pub live: usize,
}

pub enum PickerRow {
    Repo(KnownRepo),
    ThisFolder(PathBuf),
    Folder { path: PathBuf, git: bool },
}

pub struct RepoPicker {
    pub query: TextInput,
    pub choice: usize,
    pub unlisted_folders: usize,
    pub error: Option<String>,
    pub top: Cell<usize>,
    pub page: Cell<usize>,
    rows: Vec<PickerRow>,
    repos: Vec<KnownRepo>,
    home: Option<PathBuf>,
}

impl RepoPicker {
    fn new(
        query: String,
        repos: Vec<KnownRepo>,
        current: Option<&Path>,
        home: Option<PathBuf>,
    ) -> Self {
        let mut picker = Self {
            query: TextInput::new(query),
            choice: 0,
            unlisted_folders: 0,
            error: None,
            top: Cell::new(0),
            page: Cell::new(1),
            rows: Vec::new(),
            repos,
            home,
        };
        picker.refresh();
        if picker.query.text().is_empty() {
            picker.choice = picker
                .repos
                .iter()
                .position(|repo| Some(repo.path.as_path()) == current)
                .unwrap_or(0);
        }
        picker
    }

    pub fn rows(&self) -> &[PickerRow] {
        &self.rows
    }

    pub fn path_mode(&self) -> bool {
        self.typed().starts_with(['/', '~'])
    }

    fn key(&mut self, key: KeyEvent) -> PickerOutcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let page = self.page.get().max(1);
        match key.code {
            KeyCode::Esc => return PickerOutcome::Close,
            KeyCode::Enter => return self.pick(),
            KeyCode::Up => self.step(-1),
            KeyCode::Down => self.step(1),
            KeyCode::Char('p') if ctrl => self.step(-1),
            KeyCode::Char('n') if ctrl => self.step(1),
            KeyCode::PageUp => self.step(-(page as isize)),
            KeyCode::PageDown => self.step(page as isize),
            KeyCode::Tab => self.descend(),
            _ => {
                if self.query.key(key) {
                    self.refresh();
                }
            }
        }
        PickerOutcome::Stay
    }

    fn paste(&mut self, text: &str) {
        self.query.insert(text);
        self.refresh();
    }

    fn step(&mut self, by: isize) {
        let last = self.rows.len().saturating_sub(1);
        self.choice = self.choice.saturating_add_signed(by).min(last);
    }

    fn descend(&mut self) {
        let query = match self.rows.get(self.choice) {
            Some(PickerRow::Repo(repo)) => format!("{}/", repo.path.display()),
            Some(PickerRow::Folder { path, .. }) => {
                let typed = self.typed_path();
                let parent = typed.rfind('/').map_or(typed, |at| &typed[..=at]);
                format!("{parent}{}/", repo_name(path))
            }
            Some(PickerRow::ThisFolder(_)) | None => return,
        };
        self.query.set(query);
        self.refresh();
    }

    fn typed(&self) -> &str {
        self.query.text().trim()
    }

    fn refresh(&mut self) {
        self.choice = 0;
        self.error = None;
        self.top.set(0);
        self.unlisted_folders = 0;
        self.rows = match self.path_mode() {
            true => self.folders(),
            false => self.matches(),
        };
    }

    fn matches(&self) -> Vec<PickerRow> {
        let query = self.typed().to_lowercase();
        let mut ranked: Vec<(u8, &KnownRepo)> = self
            .repos
            .iter()
            .filter_map(|repo| Some((rank(&repo.path, &query)?, repo)))
            .collect();
        ranked.sort_by_key(|(rank, _)| *rank);
        ranked
            .into_iter()
            .map(|(_, repo)| PickerRow::Repo(repo.clone()))
            .collect()
    }

    fn typed_path(&self) -> &str {
        match self.typed() {
            "~" => "~/",
            typed => typed,
        }
    }

    fn expanded(&self) -> Option<String> {
        let typed = self.typed_path();
        match typed.strip_prefix('~') {
            None => Some(typed.to_string()),
            Some(rest) if rest.starts_with('/') => {
                let home = self.home.as_ref()?.display().to_string();
                Some(format!("{}{}", home.trim_end_matches('/'), rest))
            }
            Some(_) => None,
        }
    }

    fn folders(&mut self) -> Vec<PickerRow> {
        let Some(expanded) = self.expanded() else {
            return Vec::new();
        };
        let split = expanded.rfind('/').map_or(0, |at| at + 1);
        let (dir, segment) = expanded.split_at(split);
        let dir = PathBuf::from(dir);
        let wanted = segment.to_lowercase();
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| !name.starts_with('.') || segment.starts_with('.'))
            .filter(|name| name.to_lowercase().starts_with(&wanted))
            .collect();
        names.sort_by_key(|name| name.to_lowercase());
        self.unlisted_folders = names.len().saturating_sub(MAX_FOLDERS);
        let mut rows = Vec::new();
        if segment.is_empty() {
            rows.push(PickerRow::ThisFolder(dir.clone()));
        }
        rows.extend(names.into_iter().take(MAX_FOLDERS).map(|name| {
            let path = dir.join(name);
            PickerRow::Folder {
                git: path.join(".git").exists(),
                path,
            }
        }));
        rows
    }

    fn pick(&mut self) -> PickerOutcome {
        let path = match self.rows.get(self.choice) {
            Some(PickerRow::Repo(repo)) => {
                return PickerOutcome::Pick {
                    root: repo.path.clone(),
                    moved: false,
                };
            }
            Some(PickerRow::ThisFolder(path) | PickerRow::Folder { path, .. }) => path.clone(),
            None if self.path_mode() => match self.expanded() {
                Some(expanded) => PathBuf::from(expanded),
                None => return self.refuse(),
            },
            None => return PickerOutcome::Stay,
        };
        let Ok(repo) = Repo::open(&path) else {
            return self.refuse();
        };
        let moved = path.canonicalize().ok().as_deref() != Some(repo.root());
        PickerOutcome::Pick {
            root: repo.root().to_path_buf(),
            moved,
        }
    }

    fn refuse(&mut self) -> PickerOutcome {
        self.error = Some("not a git repository".into());
        PickerOutcome::Stay
    }
}

fn rank(repo: &Path, query: &str) -> Option<u8> {
    let name = repo_name(repo).to_lowercase();
    let path = repo.to_string_lossy().to_lowercase();
    if name.starts_with(query) {
        Some(0)
    } else if name.contains(query) {
        Some(1)
    } else if path.contains(query) {
        Some(2)
    } else {
        let mut chars = path.chars();
        query
            .chars()
            .all(|wanted| chars.any(|c| c == wanted))
            .then_some(3)
    }
}
