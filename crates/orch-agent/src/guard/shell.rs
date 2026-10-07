use std::iter::Peekable;
use std::str::Chars;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Word {
    pub text: String,
    pub dynamic: bool,
}

#[derive(Debug, Default)]
pub(crate) struct SimpleCommand {
    pub words: Vec<Word>,
    pub written: Vec<Word>,
    pub nested: Vec<String>,
}

impl SimpleCommand {
    pub(crate) fn invocation(&self) -> Vec<&Word> {
        const WRAPPERS: [&str; 6] = ["env", "sudo", "command", "exec", "nohup", "time"];
        self.words
            .iter()
            .skip_while(|word| word.text.contains('=') || WRAPPERS.contains(&word.text.as_str()))
            .collect()
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
}

impl Parser {
    fn word(&mut self) -> &mut Word {
        self.word.get_or_insert_with(|| Word {
            text: String::new(),
            dynamic: false,
        })
    }

    fn push(&mut self, c: char) {
        self.word().text.push(c);
    }

    fn push_dynamic(&mut self) {
        let word = self.word();
        word.text.push('$');
        word.dynamic = true;
    }

    fn end_word(&mut self) {
        let Some(word) = self.word.take() else { return };
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
                '\\' => self.escaped(chars),
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
            c => parser.push(c),
        }
    }
    parser.end_command();
    parser.commands
}
