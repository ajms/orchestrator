use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const SCROLL_STEP: u16 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    pub lines: Vec<String>,
}

impl FileDiff {
    pub fn added(&self) -> usize {
        self.lines
            .iter()
            .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
            .count()
    }

    pub fn removed(&self) -> usize {
        self.lines
            .iter()
            .filter(|line| line.starts_with('-') && !line.starts_with("---"))
            .count()
    }
}

pub(crate) fn parse_unified_diff(diff: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    for line in diff.lines() {
        if let Some(header) = line.strip_prefix("diff --git ") {
            let path = header
                .split_once(" b/")
                .map_or(header, |(_, path)| path)
                .to_string();
            files.push(FileDiff {
                path,
                lines: Vec::new(),
            });
        } else if let Some(file) = files.last_mut()
            && (line.starts_with("@@") || file.lines.iter().any(|line| line.starts_with("@@")))
        {
            file.lines.push(line.to_string());
        }
    }
    files
}

pub(crate) enum ReviewAction {
    Stay,
    Close,
    CommandLine,
}

pub(crate) struct ReviewView {
    pub base: String,
    pub files: Vec<FileDiff>,
    pub file: usize,
    pub scroll: u16,
}

impl ReviewView {
    pub fn new(base: String, files: Vec<FileDiff>) -> Self {
        Self {
            base,
            files,
            file: 0,
            scroll: 0,
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> ReviewAction {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let last = self.files.len().saturating_sub(1);
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return ReviewAction::Close,
            KeyCode::Char(':') => return ReviewAction::CommandLine,
            KeyCode::Char('d') if ctrl => self.scroll = self.scroll.saturating_add(SCROLL_STEP),
            KeyCode::Char('u') if ctrl => self.scroll = self.scroll.saturating_sub(SCROLL_STEP),
            KeyCode::Char('j') | KeyCode::Down => self.show_file((self.file + 1).min(last)),
            KeyCode::Char('k') | KeyCode::Up => self.show_file(self.file.saturating_sub(1)),
            _ => {}
        }
        ReviewAction::Stay
    }

    fn show_file(&mut self, file: usize) {
        self.file = file;
        self.scroll = 0;
    }
}
