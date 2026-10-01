use std::ops::Deref;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Default)]
pub struct TextInput {
    text: String,
    cursor: usize,
}

impl TextInput {
    pub fn new(text: String) -> Self {
        let cursor = text.len();
        Self { text, cursor }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set(&mut self, text: String) {
        self.cursor = text.len();
        self.text = text;
    }

    pub fn insert(&mut self, text: &str) {
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    pub fn key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Char(c) if !ctrl && !alt => {
                self.insert(c.encode_utf8(&mut [0; 4]));
                return true;
            }
            KeyCode::Left if ctrl => self.cursor = self.word_left(),
            KeyCode::Right if ctrl => self.cursor = self.word_right(),
            KeyCode::Char('b') if alt => self.cursor = self.word_left(),
            KeyCode::Char('f') if alt => self.cursor = self.word_right(),
            KeyCode::Left => self.cursor = self.prev_char(),
            KeyCode::Right => self.cursor = self.next_char(),
            KeyCode::Home => self.cursor = self.line_start(),
            KeyCode::End => self.cursor = self.line_end(),
            KeyCode::Char('a') if ctrl => self.cursor = self.line_start(),
            KeyCode::Char('e') if ctrl => self.cursor = self.line_end(),
            KeyCode::Backspace => return self.delete_to(self.prev_char()),
            KeyCode::Delete => return self.delete_to(self.next_char()),
            KeyCode::Char('w') if ctrl => return self.delete_to(self.rubout_start()),
            KeyCode::Char('u') if ctrl => return self.delete_to(self.line_start()),
            KeyCode::Char('k') if ctrl => return self.delete_to(self.line_end()),
            _ => {}
        }
        false
    }

    pub fn up(&mut self) {
        let start = self.line_start();
        if start == 0 {
            self.cursor = 0;
            return;
        }
        let column = self.text[start..self.cursor].chars().count();
        let above = self.text[..start - 1].rfind('\n').map_or(0, |at| at + 1);
        self.cursor = self.at_column(above, start - 1, column);
    }

    pub fn down(&mut self) {
        let end = self.line_end();
        if end == self.text.len() {
            self.cursor = end;
            return;
        }
        let column = self.text[self.line_start()..self.cursor].chars().count();
        let below = end + 1;
        let below_end = self.text[below..]
            .find('\n')
            .map_or(self.text.len(), |at| below + at);
        self.cursor = self.at_column(below, below_end, column);
    }

    fn at_column(&self, start: usize, end: usize, column: usize) -> usize {
        self.text[start..end]
            .char_indices()
            .nth(column)
            .map_or(end, |(at, _)| start + at)
    }

    fn delete_to(&mut self, at: usize) -> bool {
        let (from, to) = (at.min(self.cursor), at.max(self.cursor));
        self.text.replace_range(from..to, "");
        self.cursor = from;
        from < to
    }

    fn prev_char(&self) -> usize {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(at, _)| at)
    }

    fn next_char(&self) -> usize {
        self.text[self.cursor..]
            .chars()
            .next()
            .map_or(self.cursor, |c| self.cursor + c.len_utf8())
    }

    fn line_start(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map_or(0, |at| at + 1)
    }

    fn line_end(&self) -> usize {
        self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |at| self.cursor + at)
    }

    fn word_left(&self) -> usize {
        back_over(
            &self.text,
            self.cursor,
            |c| !c.is_alphanumeric(),
            char::is_alphanumeric,
        )
    }

    fn word_right(&self) -> usize {
        let rest = &self.text[self.cursor..];
        let gap = rest.find(char::is_alphanumeric).unwrap_or(rest.len());
        let word = rest[gap..]
            .find(|c: char| !c.is_alphanumeric())
            .unwrap_or(rest.len() - gap);
        self.cursor + gap + word
    }

    fn rubout_start(&self) -> usize {
        back_over(&self.text, self.cursor, char::is_whitespace, |c| {
            !c.is_whitespace()
        })
    }
}

fn back_over(
    text: &str,
    from: usize,
    skip: impl Fn(char) -> bool,
    word: impl Fn(char) -> bool,
) -> usize {
    let mut chars = text[..from].char_indices().rev().peekable();
    let mut at = from;
    while let Some((index, _)) = chars.next_if(|(_, c)| skip(*c)) {
        at = index;
    }
    while let Some((index, _)) = chars.next_if(|(_, c)| word(*c)) {
        at = index;
    }
    at
}

#[derive(Debug, Clone, Default)]
pub struct Suggested {
    input: TextInput,
    suggested: bool,
}

impl Suggested {
    pub fn suggestion(text: String) -> Self {
        Self {
            input: TextInput::new(text),
            suggested: true,
        }
    }

    pub fn suggested(&self) -> bool {
        self.suggested
    }

    pub fn suggest(&mut self, text: String) {
        self.input.set(text);
        self.suggested = true;
    }

    pub fn set(&mut self, text: String) {
        self.input.set(text);
        self.suggested = false;
    }

    pub fn accept(&mut self) {
        self.suggested = false;
    }

    pub fn replace(&mut self, text: &str) {
        if std::mem::take(&mut self.suggested) {
            self.input.set(String::new());
        }
        self.input.insert(text);
    }

    pub fn key(&mut self, key: KeyEvent) -> bool {
        let changed = self.input.key(key);
        if changed {
            self.suggested = false;
        }
        changed
    }
}

impl Deref for Suggested {
    type Target = TextInput;

    fn deref(&self) -> &TextInput {
        &self.input
    }
}
