// PROTOTYPE, throwaway: four layouts of the :new popup over the real TUI.
// cargo run -p orch-tui --example prototype_new_popup [-- A|B|C|D]
// F7/F8 switch variant · F2 long prompt · F3 clear · Ctrl+r picker · Tab field · Esc quit

use std::io;
use std::path::PathBuf;

use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event as TermEvent, KeyCode, KeyEvent,
    KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use orch_core::SessionId;
use orch_protocol::{AgentStateView, FlagsView, FromDaemon, PhaseView, Request, SessionView, Size};
use orch_tui::{DaemonLink, Event, PaneId, RequestId, Tui, TuiConfig};
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

const VARIANTS: [(&str, &str); 4] = [
    ("A", "Compact dialog, prompt box"),
    ("B", "Full-height editor sheet"),
    ("C", "Two columns: fields | prompt"),
    ("D", "Bottom drawer, chat-style"),
];

const LONG: &str = "Rework the new session popup. The repo is usually wrong when it opens, the key navigation is clumsy and long prompts look broken.\n\nRequirements:\n- start on the selected Session's Repo\n- a filterable Repo picker with path completion, so I can type ~/repos/da and Tab into dagster-dp\n- proper cursor movement in every text field: left/right, home/end, word jumps, ctrl+w\n- the prompt wraps and scrolls instead of growing past the screen\n- pasting works in the prompt\n\nAlso, after creating a Session the preparation takes a few seconds; show a Preparing row in the sidebar with elapsed time so it's clear something is happening. If it fails, keep the row red with the reason and let me reopen the form with everything filled in.\n\nPlease write tests first for each of these, then implement, then refactor. Keep the code style consistent with the rest of orch-tui and don't add docstrings.";

const REPOS: [&str; 5] = [
    "~/repos/ajms/orchestrator",
    "~/repos/webshop",
    "~/repos/dagster-dp",
    "~/repos/dagster-dp-fresh",
    "~/repos/data-product-sandbox",
];

#[derive(Clone, Copy, PartialEq)]
enum Field {
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

struct Proto {
    variant: usize,
    field: Field,
    prompt: String,
    repo: usize,
    picker: Option<String>,
    choice: usize,
}

impl Proto {
    fn branch(&self) -> String {
        let slug: String = self
            .prompt
            .split_whitespace()
            .take(5)
            .map(|word| word.to_lowercase())
            .map(|word| {
                word.chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("-");
        format!("orch/{slug}")
    }

    fn matches(&self) -> Vec<&'static str> {
        let query = self.picker.as_deref().unwrap_or("");
        REPOS
            .iter()
            .copied()
            .filter(|repo| {
                repo.contains(query.trim_start_matches("~/repos/").trim_end_matches('/'))
            })
            .collect()
    }

    fn key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let matches = self.matches();
        if let Some(query) = &mut self.picker {
            match key.code {
                KeyCode::Esc => self.picker = None,
                KeyCode::Down | KeyCode::Char('n') if key.code == KeyCode::Down || ctrl => {
                    self.choice = (self.choice + 1).min(matches.len().saturating_sub(1));
                }
                KeyCode::Up | KeyCode::Char('p') if key.code == KeyCode::Up || ctrl => {
                    self.choice = self.choice.saturating_sub(1);
                }
                KeyCode::Tab => {
                    if let Some(chosen) = matches.get(self.choice) {
                        *query = format!("{chosen}/");
                        self.choice = 0;
                    }
                }
                KeyCode::Enter => {
                    if let Some(chosen) = matches.get(self.choice) {
                        self.repo = REPOS.iter().position(|repo| repo == chosen).unwrap_or(0);
                    }
                    self.choice = 0;
                    self.picker = None;
                    self.field = Field::Prompt;
                }
                KeyCode::Backspace => {
                    query.pop();
                    self.choice = 0;
                }
                KeyCode::Char(c) => {
                    query.push(c);
                    self.choice = 0;
                }
                _ => {}
            }
            return true;
        }
        match key.code {
            KeyCode::Esc => return false,
            KeyCode::F(7) => self.variant = (self.variant + VARIANTS.len() - 1) % VARIANTS.len(),
            KeyCode::F(8) => self.variant = (self.variant + 1) % VARIANTS.len(),
            KeyCode::F(2) => self.prompt = LONG.into(),
            KeyCode::F(3) => self.prompt.clear(),
            KeyCode::Char('r') if ctrl => self.picker = Some(String::new()),
            KeyCode::Tab => {
                let at = FIELDS.iter().position(|f| *f == self.field).unwrap();
                self.field = FIELDS[(at + 1) % FIELDS.len()];
            }
            KeyCode::BackTab => {
                let at = FIELDS.iter().position(|f| *f == self.field).unwrap();
                self.field = FIELDS[(at + FIELDS.len() - 1) % FIELDS.len()];
            }
            KeyCode::Enter if self.field == Field::Repo => self.picker = Some(String::new()),
            KeyCode::Enter if self.field == Field::Prompt => self.prompt.push('\n'),
            KeyCode::Backspace if self.field == Field::Prompt => {
                self.prompt.pop();
            }
            KeyCode::Char(c) if !ctrl && self.field == Field::Prompt => self.prompt.push(c),
            _ => {}
        }
        true
    }
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut rows = Vec::new();
    for line in text.split('\n') {
        let mut row = String::new();
        for word in line.split(' ') {
            let wanted = if row.is_empty() {
                word.width()
            } else {
                row.width() + 1 + word.width()
            };
            if wanted > width && !row.is_empty() {
                rows.push(std::mem::take(&mut row));
            }
            if !row.is_empty() {
                row.push(' ');
            }
            row.push_str(word);
            while row.width() > width {
                let cut: String = row.chars().take(width).collect();
                row = row.chars().skip(width).collect();
                rows.push(cut);
            }
        }
        rows.push(row);
    }
    rows
}

fn prompt_lines(proto: &Proto, width: usize, height: usize, focused: bool) -> Vec<Line<'static>> {
    if proto.prompt.is_empty() && !focused {
        return vec![Line::from("What should the Agent do?").dark_gray()];
    }
    let mut rows = wrap(&proto.prompt, width.saturating_sub(1));
    if focused {
        let last = rows.last_mut().unwrap();
        last.push('▏');
    }
    let total = rows.len();
    let skip = total.saturating_sub(height);
    let mut lines: Vec<Line> = rows.into_iter().skip(skip).map(Line::from).collect();
    if skip > 0 {
        lines[0] = Line::from(format!("↑ {skip} more lines"))
            .dark_gray()
            .italic();
    }
    lines
}

fn label(proto: &Proto, field: Field, name: &str, width: usize) -> Span<'static> {
    let style = match proto.field == field {
        true => Style::new().fg(Color::Black).bg(Color::Cyan),
        false => Style::new().fg(Color::DarkGray),
    };
    Span::styled(format!(" {name:<width$} "), style)
}

fn hint(proto: &Proto) -> &'static str {
    if proto.picker.is_some() {
        return " type to filter · ↑/↓ choose · Tab complete · Enter pick · Esc back";
    }
    match proto.field {
        Field::Repo => " Enter/Ctrl+r pick Repo · Tab next · Ctrl+s create · Esc cancel",
        Field::Prompt => {
            " Enter newline · Ctrl+g $EDITOR · Tab options · Ctrl+s create · Esc cancel"
        }
        Field::Branch | Field::Base => " ←/→ cursor · ↑/↓ suggestions · Tab next · Ctrl+s create",
        Field::Preset => " ←/→ choose · Tab next · Ctrl+s create · Esc cancel",
    }
}

fn picker_lines(proto: &Proto) -> Vec<Line<'static>> {
    let query = proto.picker.clone().unwrap_or_default();
    let mut lines = vec![Line::from(vec![
        Span::raw(" > ").cyan(),
        Span::raw(format!("{query}▏")),
    ])];
    for (at, repo) in proto.matches().into_iter().enumerate() {
        let name = repo.rsplit('/').next().unwrap_or(repo);
        let chosen = at == proto.choice;
        let line = Line::from(format!(
            " {} {name:<22} {repo}",
            if chosen { "▸" } else { " " }
        ));
        lines.push(match chosen {
            true => line.style(Style::new().bg(Color::Rgb(50, 50, 70))),
            false => line,
        });
    }
    lines
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .areas(area);
    area
}

fn frame_block(title: String) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Cyan))
        .title(title)
}

fn variant_a(proto: &Proto, frame: &mut Frame) {
    let screen = frame.area();
    let width = screen.width.saturating_sub(8).min(96);
    let inner = usize::from(width.saturating_sub(4));
    let max_prompt = usize::from(screen.height.saturating_sub(14)).clamp(3, 16);
    let rows = wrap(&proto.prompt, inner).len().clamp(3, max_prompt);
    let picker = proto.picker.is_some().then(|| picker_lines(proto));
    let picker_rows = picker.as_ref().map_or(0, |lines| lines.len() as u16 + 2);
    let height = rows as u16 + 2 + 7 + picker_rows;
    let area = centered(screen, width, height);
    frame.render_widget(Clear, area);
    let repo = REPOS[proto.repo];
    let title = Line::from(vec![
        Span::raw(" :new Session in "),
        Span::styled(
            format!("{repo} ▾"),
            match proto.field {
                Field::Repo => Style::new().fg(Color::Black).bg(Color::Cyan),
                _ => Style::new().bold(),
            },
        ),
        Span::raw(" "),
    ]);
    let block = frame_block(String::new())
        .title(title)
        .title_bottom(Line::from(hint(proto)).dark_gray());
    let body = block.inner(area);
    frame.render_widget(block, area);
    let [picker_area, prompt_area, options] = Layout::vertical([
        Constraint::Length(picker_rows),
        Constraint::Length(rows as u16 + 2),
        Constraint::Min(0),
    ])
    .areas(body);
    if let Some(lines) = picker {
        let block = Block::bordered().border_style(Style::new().fg(Color::DarkGray));
        frame.render_widget(Paragraph::new(lines).block(block), picker_area);
    }
    let focused = proto.field == Field::Prompt;
    let border = match focused {
        true => Style::new().fg(Color::Cyan),
        false => Style::new().fg(Color::DarkGray),
    };
    let block = Block::bordered().border_style(border).title(" Prompt ");
    frame.render_widget(
        Paragraph::new(prompt_lines(proto, inner, rows, focused)).block(block),
        prompt_area,
    );
    let lines = vec![
        Line::default(),
        Line::from(vec![
            label(proto, Field::Branch, "Branch", 6),
            Span::raw(format!(" {}", proto.branch())),
        ]),
        Line::from(vec![
            label(proto, Field::Base, "Base", 6),
            Span::raw(" main "),
            Span::raw("(Repo default)").dark_gray(),
        ]),
        Line::from(vec![
            label(proto, Field::Preset, "Preset", 6),
            Span::raw(" ◂ default ▸"),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), options);
}

fn variant_b(proto: &Proto, frame: &mut Frame) {
    let screen = frame.area();
    let area = Rect {
        x: screen.x + 41.min(screen.width / 3),
        y: screen.y,
        width: screen.width.saturating_sub(41.min(screen.width / 3)),
        height: screen.height.saturating_sub(1),
    };
    frame.render_widget(Clear, area);
    let block = frame_block(" :new Session ".into());
    let body = block.inner(area);
    frame.render_widget(block, area);
    let [crumbs, rule, main, hints] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(body);
    let sep = || Span::raw("  ›  ").dark_gray();
    let chip = |field: Field, text: String| match proto.field == field {
        true => Span::styled(text, Style::new().fg(Color::Black).bg(Color::Cyan)),
        false => Span::raw(text),
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            chip(Field::Repo, format!(" {} ", REPOS[proto.repo])),
            sep(),
            chip(Field::Base, " from main ".into()),
            sep(),
            chip(Field::Branch, format!(" {} ", proto.branch())),
            sep(),
            chip(Field::Preset, " preset: default ".into()),
        ])),
        crumbs,
    );
    frame.render_widget(
        Paragraph::new("─".repeat(usize::from(rule.width))).dark_gray(),
        rule,
    );
    let main = match proto.picker.is_some() {
        true => {
            let [list, rest] =
                Layout::horizontal([Constraint::Length(48), Constraint::Min(1)]).areas(main);
            let block = Block::new()
                .borders(Borders::RIGHT)
                .border_style(Style::new().fg(Color::DarkGray));
            frame.render_widget(Paragraph::new(picker_lines(proto)).block(block), list);
            rest
        }
        false => main,
    };
    let focused = proto.field == Field::Prompt;
    let width = usize::from(main.width.saturating_sub(2));
    let lines = prompt_lines(proto, width, usize::from(main.height), focused);
    frame.render_widget(
        Paragraph::new(lines),
        main.inner(ratatui::layout::Margin::new(1, 0)),
    );
    frame.render_widget(Paragraph::new(hint(proto)).dark_gray(), hints);
}

fn variant_c(proto: &Proto, frame: &mut Frame) {
    let screen = frame.area();
    let width = screen.width.saturating_sub(6).min(130);
    let height = screen.height.saturating_sub(4).min(26);
    let area = centered(screen, width, height);
    frame.render_widget(Clear, area);
    let block =
        frame_block(" :new Session ".into()).title_bottom(Line::from(hint(proto)).dark_gray());
    let body = block.inner(area);
    frame.render_widget(block, area);
    let left_width = match proto.picker.is_some() {
        true => body.width * 3 / 5,
        false => 40,
    };
    let [left, right] =
        Layout::horizontal([Constraint::Length(left_width), Constraint::Min(20)]).areas(body);
    let left_block = Block::new()
        .borders(Borders::RIGHT)
        .border_style(Style::new().fg(Color::DarkGray));
    let lines = match proto.picker.is_some() {
        true => picker_lines(proto),
        false => {
            let field = |f: Field, name: &str, value: String, note: &str| {
                vec![
                    Line::from(label(proto, f, name, 36)),
                    Line::from(vec![
                        Span::raw(format!("  {value}")),
                        Span::raw(note.to_string()).dark_gray(),
                    ]),
                    Line::default(),
                ]
            };
            [
                field(Field::Repo, "Repo", REPOS[proto.repo].into(), ""),
                field(Field::Branch, "Branch", proto.branch(), ""),
                field(Field::Base, "Base", "main".into(), "  Repo default"),
                field(Field::Preset, "Preset", "◂ default ▸".into(), ""),
            ]
            .concat()
        }
    };
    frame.render_widget(Paragraph::new(lines).block(left_block), left);
    let focused = proto.field == Field::Prompt;
    let [head, text] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(right);
    frame.render_widget(
        Paragraph::new(Line::from(label(proto, Field::Prompt, "Prompt", 6))),
        head,
    );
    let inner = text.inner(ratatui::layout::Margin::new(1, 0));
    let lines = prompt_lines(
        proto,
        usize::from(inner.width),
        usize::from(inner.height),
        focused,
    );
    frame.render_widget(Paragraph::new(lines), inner);
}

fn variant_d(proto: &Proto, frame: &mut Frame) {
    let screen = frame.area();
    let inner = usize::from(screen.width.saturating_sub(4));
    let max_prompt = usize::from(screen.height / 2).max(3);
    let rows = wrap(&proto.prompt, inner).len().clamp(1, max_prompt);
    let picker = proto.picker.is_some().then(|| picker_lines(proto));
    let picker_rows = picker.as_ref().map_or(0, |lines| lines.len() as u16);
    let height = (rows as u16 + 5 + picker_rows).min(screen.height - 1);
    let area = Rect {
        x: screen.x,
        y: screen.bottom().saturating_sub(height + 1),
        width: screen.width,
        height,
    };
    frame.render_widget(Clear, area);
    let block = Block::new()
        .borders(Borders::TOP)
        .border_style(Style::new().fg(Color::Cyan))
        .title(" :new Session ");
    let body = block.inner(area);
    frame.render_widget(block, area);
    let [picker_area, chips, prompt, hints] = Layout::vertical([
        Constraint::Length(picker_rows),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(body);
    if let Some(lines) = picker {
        frame.render_widget(Paragraph::new(lines), picker_area);
    }
    let chip = |field: Field, text: String| {
        let style = match proto.field == field {
            true => Style::new().fg(Color::Black).bg(Color::Cyan),
            false => Style::new().bg(Color::Rgb(50, 50, 70)),
        };
        vec![Span::styled(format!(" {text} "), style), Span::raw(" ")]
    };
    frame.render_widget(
        Paragraph::new(Line::from(
            [
                chip(
                    Field::Repo,
                    format!("⌂ {}", REPOS[proto.repo].rsplit('/').next().unwrap()),
                ),
                chip(Field::Branch, format!("⎇ {}", proto.branch())),
                chip(Field::Base, "⇡ main".into()),
                chip(Field::Preset, "◈ default".into()),
            ]
            .concat(),
        )),
        chips,
    );
    let focused = proto.field == Field::Prompt;
    let mut lines = prompt_lines(proto, inner, usize::from(prompt.height), focused);
    if let Some(first) = lines.first_mut() {
        first
            .spans
            .insert(0, Span::raw("❯ ").cyan().add_modifier(Modifier::BOLD));
    }
    frame.render_widget(
        Paragraph::new(lines),
        prompt.inner(ratatui::layout::Margin::new(1, 0)),
    );
    frame.render_widget(Paragraph::new(hint(proto)).dark_gray(), hints);
}

fn switcher(proto: &Proto, frame: &mut Frame) {
    let (key, name) = VARIANTS[proto.variant];
    let text = format!(" ◂ F7   {key} ({name})   F8 ▸ ");
    let width = text.width() as u16;
    let screen = frame.area();
    let area = Rect {
        x: screen.x + screen.width.saturating_sub(width) / 2,
        y: screen.bottom().saturating_sub(1),
        width: width.min(screen.width),
        height: 1,
    };
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(text).style(Style::new().fg(Color::Black).bg(Color::Magenta).bold()),
        area,
    );
}

struct NoDaemon;

impl DaemonLink for NoDaemon {
    fn request(&mut self, _: RequestId, _: Request) {}
    fn open_pane(&mut self, _: PaneId, _: &SessionId, _: Size) {}
    fn close_pane(&mut self) {}
    fn input(&mut self, _: Vec<u8>) {}
    fn paste(&mut self, _: String) {}
    fn resize_pane(&mut self, _: Size) {}
}

fn session(repo: &str, slug: &str, agent: AgentStateView) -> SessionView {
    let repo = PathBuf::from(repo);
    SessionView {
        id: SessionId(slug.into()),
        worktree: repo.join(".orchestrator/worktrees").join(slug),
        repo,
        slug: slug.into(),
        branch: format!("orch/{slug}"),
        base: "main".into(),
        phase: PhaseView::Active,
        agent: Some(agent),
        flags: FlagsView::default(),
        preset: "default".into(),
        mode: None,
        conversation: None,
        port_base: None,
        port_size: None,
        setup_output: None,
        error: None,
        holder_pid: None,
        guards_enabled: true,
        guard_prompts: Vec::new(),
        context_used_percent: Some(34.0),
        cost_usd: None,
        subagents: Vec::new(),
    }
}

fn main() -> io::Result<()> {
    let start = std::env::args()
        .nth(1)
        .and_then(|key| {
            VARIANTS
                .iter()
                .position(|(k, _)| k.eq_ignore_ascii_case(&key))
        })
        .unwrap_or(0);
    let dump = std::env::args().any(|arg| arg == "dump");
    let (cols, rows) = match dump {
        true => (120, 30),
        false => crossterm::terminal::size()?,
    };
    let mut tui = Tui::new(TuiConfig::default(), NoDaemon, Size { rows, cols });
    let sessions = vec![
        session(
            "/home/me/repos/ajms/orchestrator",
            "rework-new-popup",
            AgentStateView::Working,
        ),
        session(
            "/home/me/repos/ajms/orchestrator",
            "release-workflow",
            AgentStateView::Idle,
        ),
        session(
            "/home/me/repos/webshop",
            "fix-login-timeout",
            AgentStateView::NeedsInput,
        ),
        session(
            "/home/me/repos/dagster-dp",
            "partition-backfill",
            AgentStateView::Idle,
        ),
    ];
    tui.handle(Event::Daemon(FromDaemon::Sessions { sessions }));
    let mut proto = Proto {
        variant: start,
        field: Field::Prompt,
        prompt: String::new(),
        repo: 0,
        picker: None,
        choice: 0,
    };
    if dump {
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(cols, rows)).unwrap();
        for variant in 0..VARIANTS.len() {
            for (prompt, picker) in [(LONG, false), ("Fix the login timeout", true)] {
                proto.variant = variant;
                proto.prompt = prompt.into();
                proto.picker = picker.then(|| "da".to_string());
                proto.choice = usize::from(picker);
                terminal
                    .draw(|frame| {
                        tui.render(frame);
                        match proto.variant {
                            0 => variant_a(&proto, frame),
                            1 => variant_b(&proto, frame),
                            2 => variant_c(&proto, frame),
                            _ => variant_d(&proto, frame),
                        }
                        switcher(&proto, frame);
                    })
                    .unwrap();
                let buffer = terminal.backend().buffer().clone();
                for y in 0..rows {
                    let line: String = (0..cols)
                        .map(|x| buffer[(x, y)].symbol().to_string())
                        .collect();
                    println!("{line}");
                }
                println!();
            }
        }
        return Ok(());
    }
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let result = (|| -> io::Result<()> {
        loop {
            terminal.draw(|frame| {
                tui.render(frame);
                match proto.variant {
                    0 => variant_a(&proto, frame),
                    1 => variant_b(&proto, frame),
                    2 => variant_c(&proto, frame),
                    _ => variant_d(&proto, frame),
                }
                switcher(&proto, frame);
            })?;
            match event::read()? {
                TermEvent::Key(key) if !proto.key(key) => return Ok(()),
                TermEvent::Paste(text) if proto.field == Field::Prompt => {
                    proto.prompt.push_str(&text)
                }
                TermEvent::Resize(cols, rows) => {
                    tui.handle(Event::Terminal(TermEvent::Resize(cols, rows)));
                }
                _ => {}
            }
        }
    })();
    execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen)?;
    disable_raw_mode()?;
    result
}
