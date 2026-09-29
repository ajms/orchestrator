use std::iter::Peekable;
use std::str::Chars;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Word {
    pub text: String,
    pub dynamic: bool,
}

#[derive(Debug, Default)]
pub(super) struct SimpleCommand {
    pub words: Vec<Word>,
    pub written: Vec<Word>,
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

    fn double_quoted(&mut self, chars: &mut Peekable<Chars>) {
        self.word();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        self.push(escaped);
                    }
                }
                '$' | '`' => self.push_dynamic(),
                c => self.push(c),
            }
        }
    }
}

fn skip_substitution(chars: &mut Peekable<Chars>) {
    let mut depth = 0;
    for c in chars.by_ref() {
        match c {
            '(' => depth += 1,
            ')' if depth == 1 => break,
            ')' => depth -= 1,
            _ => {}
        }
    }
}

fn eat(chars: &mut Peekable<Chars>, expected: char) -> bool {
    chars.next_if_eq(&expected).is_some()
}

pub(super) fn parse(script: &str) -> Vec<SimpleCommand> {
    let mut parser = Parser::default();
    let mut chars = script.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => parser.single_quoted(&mut chars),
            '"' => parser.double_quoted(&mut chars),
            '\\' => {
                if let Some(escaped) = chars.next() {
                    parser.push(escaped);
                }
            }
            '$' | '`' => {
                parser.push_dynamic();
                if c == '$' && chars.peek() == Some(&'(') {
                    skip_substitution(&mut chars);
                }
            }
            '>' => {
                eat(&mut chars, '>');
                let duplicates_fd = eat(&mut chars, '&');
                parser.start_redirect(if duplicates_fd {
                    Redirect::Ignore
                } else {
                    Redirect::Write
                });
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
