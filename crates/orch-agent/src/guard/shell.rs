use std::collections::VecDeque;
use std::iter::Peekable;
use std::str::Chars;

use super::invocations;

const MAX_CANDIDATES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Word {
    pub text: String,
    pub dynamic: bool,
    pub expanded: bool,
}

impl Word {
    pub(crate) fn candidates(&self) -> Option<Vec<String>> {
        if self.expanded || self.dynamic && self.text.starts_with('~') {
            return None;
        }
        let mut pending = VecDeque::from([self.text.clone()]);
        let mut done = Vec::new();
        while let Some(text) = pending.pop_front() {
            match braces(&text) {
                Some(alternatives) => alternatives
                    .into_iter()
                    .rev()
                    .for_each(|alternative| pending.push_front(alternative)),
                None => done.push(text),
            }
            if done.len() + pending.len() > MAX_CANDIDATES {
                return None;
            }
        }
        Some(done)
    }
}

fn braces(text: &str) -> Option<Vec<String>> {
    text.match_indices('{').find_map(|(open, _)| {
        let mut depth = 0;
        let mut commas = Vec::new();
        for (at, c) in text[open..].char_indices().map(|(at, c)| (open + at, c)) {
            match c {
                '{' => depth += 1,
                '}' if depth == 1 => {
                    if commas.is_empty() {
                        return None;
                    }
                    let (head, tail) = (&text[..open], &text[at + 1..]);
                    let bounds: Vec<usize> = std::iter::once(open)
                        .chain(commas)
                        .chain(std::iter::once(at))
                        .collect();
                    return Some(
                        bounds
                            .windows(2)
                            .map(|pair| format!("{head}{}{tail}", &text[pair[0] + 1..pair[1]]))
                            .collect(),
                    );
                }
                '}' => depth -= 1,
                ',' if depth == 1 => commas.push(at),
                _ => {}
            }
        }
        None
    })
}

#[derive(Debug, Default)]
pub(crate) struct SimpleCommand {
    pub words: Vec<Word>,
    pub written: Vec<Word>,
    pub nested: Vec<String>,
}

pub(crate) enum DirectoryChange<'w> {
    Into(&'w Word),
    Unknown,
}

impl SimpleCommand {
    pub(crate) fn invocation(&self) -> Vec<&Word> {
        let texts: Vec<&str> = self.words.iter().map(|word| word.text.as_str()).collect();
        let start = invocations::program_start(&texts, &mut Vec::new());
        self.words[start..].iter().collect()
    }

    pub(crate) fn directory_change(&self) -> Option<DirectoryChange<'_>> {
        let words = self.invocation();
        let (program, args) = words.split_first()?;
        let mut operands = args.iter().filter(|word| !word.text.starts_with('-'));
        match program.text.as_str() {
            "cd" | "pushd" => Some(match (operands.next(), operands.next()) {
                (Some(dir), None) if !dir.dynamic => DirectoryChange::Into(dir),
                _ => DirectoryChange::Unknown,
            }),
            "popd" => Some(DirectoryChange::Unknown),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Redirect {
    Write,
    Ignore,
}

#[derive(Default)]
struct Parser {
    commands: Vec<SimpleCommand>,
    current: SimpleCommand,
    word: Option<Word>,
    redirect: Option<Redirect>,
    tilde: bool,
}

const SAFE: &str = "_./:=@%+,-";

impl Parser {
    fn word(&mut self) -> &mut Word {
        self.word.get_or_insert_with(|| Word {
            text: String::new(),
            dynamic: false,
            expanded: false,
        })
    }

    fn push(&mut self, c: char) {
        self.word().text.push(c);
    }

    fn push_unquoted(&mut self, c: char) {
        let starts = self.word.as_ref().is_none_or(|word| word.text.is_empty());
        self.tilde |= starts && c == '~';
        let word = self.word();
        word.dynamic |= !(c.is_ascii_alphanumeric() || SAFE.contains(c) || c == '~');
        word.text.push(c);
    }

    fn push_dynamic(&mut self) {
        let word = self.word();
        word.text.push('$');
        word.dynamic = true;
        word.expanded = true;
    }

    fn end_word(&mut self) {
        let tilde = std::mem::take(&mut self.tilde);
        let Some(mut word) = self.word.take() else {
            return;
        };
        word.dynamic |= tilde && word.text != "~" && !word.text.starts_with("~/");
        match self.redirect.take() {
            Some(Redirect::Write) => self.current.written.push(word),
            Some(Redirect::Ignore) => {}
            None => self.current.words.push(word),
        }
    }

    fn end_command(&mut self) {
        self.end_word();
        self.redirect = None;
        let command = std::mem::take(&mut self.current);
        if !command.words.is_empty() || !command.written.is_empty() {
            self.commands.push(command);
        }
    }

    fn start_redirect(&mut self, redirect: Redirect) {
        let fd_prefix = self
            .word
            .as_ref()
            .is_some_and(|word| word.text.chars().all(|c| c.is_ascii_digit()));
        if fd_prefix {
            self.word = None;
        } else {
            self.end_word();
        }
        self.redirect = Some(redirect);
    }

    fn single_quoted(&mut self, chars: &mut Peekable<Chars>) {
        self.word();
        for c in chars.by_ref() {
            if c == '\'' {
                break;
            }
            self.push(c);
        }
    }

    fn escaped(&mut self, chars: &mut Peekable<Chars>) {
        match chars.next() {
            Some('\n') | None => {}
            Some(escaped) => self.push(escaped),
        }
    }

    fn double_quoted(&mut self, chars: &mut Peekable<Chars>) {
        self.word();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' if chars.peek() == Some(&'\n') => self.escaped(chars),
                '\\' => {
                    self.escaped(chars);
                    self.word().dynamic = true;
                }
                '$' | '`' => self.expansion(c, chars),
                c => self.push(c),
            }
        }
    }

    fn expansion(&mut self, c: char, chars: &mut Peekable<Chars>) {
        self.push_dynamic();
        let nested = match c {
            '`' => Some(backticks(chars)),
            _ if chars.peek() == Some(&'(') => Some(substitution(chars)),
            _ => None,
        };
        self.current.nested.extend(nested);
    }

    fn duplication(&mut self, chars: &mut Peekable<Chars>) {
        let mut target = String::new();
        while let Some(digit) = chars.next_if(char::is_ascii_digit) {
            target.push(digit);
        }
        let ends_word =
            |c: Option<&char>| c.is_none_or(|c| c.is_whitespace() || ";&|<>()".contains(*c));
        let closes = target.is_empty() && eat(chars, '-');
        let duplicates = closes || !target.is_empty() && ends_word(chars.peek());
        self.start_redirect(if duplicates {
            Redirect::Ignore
        } else {
            Redirect::Write
        });
        for c in target.chars() {
            self.push(c);
        }
        if closes {
            self.push('-');
        }
    }
}

fn copy_quoted(chars: &mut Peekable<Chars>, inner: &mut String, quote: char) {
    while let Some(c) = chars.next() {
        inner.push(c);
        if c == quote {
            break;
        }
        if c == '\\'
            && quote == '"'
            && let Some(escaped) = chars.next()
        {
            inner.push(escaped);
        }
    }
}

fn substitution(chars: &mut Peekable<Chars>) -> String {
    chars.next();
    let mut depth = 1;
    let mut inner = String::new();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                inner.push(c);
                inner.extend(chars.next());
                continue;
            }
            '\'' | '"' => {
                inner.push(c);
                copy_quoted(chars, &mut inner, c);
                continue;
            }
            '(' => depth += 1,
            ')' if depth == 1 => break,
            ')' => depth -= 1,
            _ => {}
        }
        inner.push(c);
    }
    inner
}

fn backticks(chars: &mut Peekable<Chars>) -> String {
    let mut inner = String::new();
    while let Some(c) = chars.next() {
        match c {
            '`' => break,
            '\\' => match chars.next() {
                Some(escaped @ ('`' | '$' | '\\')) => inner.push(escaped),
                Some(other) => {
                    inner.push('\\');
                    inner.push(other);
                }
                None => {}
            },
            c => inner.push(c),
        }
    }
    inner
}

fn eat(chars: &mut Peekable<Chars>, expected: char) -> bool {
    chars.next_if_eq(&expected).is_some()
}

pub(crate) fn parse(script: &str) -> Vec<SimpleCommand> {
    let mut parser = Parser::default();
    let mut chars = script.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => parser.single_quoted(&mut chars),
            '"' => parser.double_quoted(&mut chars),
            '\\' => parser.escaped(&mut chars),
            '$' | '`' => parser.expansion(c, &mut chars),
            '>' => {
                if !eat(&mut chars, '>') {
                    eat(&mut chars, '|');
                }
                if eat(&mut chars, '&') {
                    parser.duplication(&mut chars);
                } else {
                    parser.start_redirect(Redirect::Write);
                }
            }
            '&' if eat(&mut chars, '>') => {
                eat(&mut chars, '>');
                parser.start_redirect(Redirect::Write);
            }
            '<' => parser.start_redirect(Redirect::Ignore),
            ';' | '&' | '|' | '\n' | '(' | ')' => parser.end_command(),
            c if c.is_whitespace() => parser.end_word(),
            c => parser.push_unquoted(c),
        }
    }
    parser.end_command();
    parser.commands
}
