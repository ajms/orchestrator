use orch_protocol::{GuardKindView, GuardPrompt, LandingMode, SessionView};
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};

use crate::app::{App, Popup};
use crate::discard::DiscardConfirm;
use crate::land::LandForm;
use crate::new_form::{Field, NewForm};
use crate::sessions::repo_name;

pub(super) fn draw(app: &App, frame: &mut Frame) {
    match &app.popup {
        Some(Popup::New(form)) => new_form(frame, form),
        Some(Popup::Trust(prompt)) => trust(frame, &prompt.create.repo, &prompt.items),
        Some(Popup::Land(form)) => {
            if let Some(view) = app.sessions.get(&form.session) {
                land(frame, view, form);
            }
        }
        Some(Popup::Discard(confirm)) => {
            if let Some(view) = app.sessions.get(&confirm.session) {
                discard(frame, view, confirm);
            }
        }
        None => {}
    }
    if let Some((_, prompt)) = app.guard_prompt() {
        guard(frame, prompt);
    }
}

fn new_form(frame: &mut Frame, form: &NewForm) {
    let label = |field: Field, name: &str| {
        let style = match form.field == field {
            true => Style::new().fg(Color::Black).bg(Color::Cyan),
            false => Style::new(),
        };
        Span::styled(format!(" {name:<9} "), style)
    };
    let cursor = |field: Field| if form.field == field { "▏" } else { "" };
    let repo = match form.repos.get(form.repo) {
        Some(repo) => format!("◂ {} ▸", repo.display()),
        None => format!("◂ other path… ▸ {}{}", form.other_path, cursor(Field::Repo)),
    };
    let mut lines = vec![Line::from(vec![
        label(Field::Repo, "Repo"),
        Span::raw(repo),
    ])];
    let prompt = format!("{}{}", form.prompt, cursor(Field::Prompt));
    for (at, text) in prompt.split('\n').enumerate() {
        let head = match at {
            0 => label(Field::Prompt, "Prompt"),
            _ => Span::raw(" ".repeat(11)),
        };
        lines.push(Line::from(vec![head, Span::raw(text.to_string())]));
    }
    lines.push(Line::from(vec![
        label(Field::Branch, "Branch"),
        Span::raw(format!("{}{}", form.branch, cursor(Field::Branch))),
    ]));
    let base = match form.base.is_empty() {
        true => "(Repo default)".to_string(),
        false => form.base.clone(),
    };
    let mut base_line = vec![
        label(Field::Base, "Base"),
        Span::raw(format!("{base}{}", cursor(Field::Base))),
    ];
    if !form.base_candidates.is_empty() {
        base_line.push(Span::raw("  ◂ ▸ other Sessions' Branches").dark_gray());
    }
    lines.push(Line::from(base_line));
    let preset = match form.preset {
        Some(at) => format!("◂ {} ▸", form.presets[at]),
        None => "◂ (Repo default) ▸".to_string(),
    };
    lines.push(Line::from(vec![
        label(Field::Preset, "Preset"),
        Span::raw(preset),
    ]));
    lines.push(Line::default());
    if let Some(error) = &form.error {
        lines.push(Line::from(format!(" {error}")).red());
    }
    lines.push(
        Line::from(" Tab field · ←/→ choose · Ctrl+g $EDITOR · Ctrl+s create · Esc cancel")
            .dark_gray(),
    );
    show(frame, " :new Session ", Color::Cyan, lines, 84);
}

fn land(frame: &mut Frame, view: &SessionView, form: &LandForm) {
    let squash = form.mode == LandingMode::Squash;
    let choice = |text: String, chosen: bool| match chosen {
        true => Span::styled(text, Style::new().fg(Color::Black).bg(Color::Green)),
        false => Span::raw(text),
    };
    let mut lines = vec![
        Line::from(format!(
            " Land {} / {} into {}",
            repo_name(&view.repo),
            view.slug,
            view.base
        ))
        .bold(),
        Line::default(),
        Line::from(vec![
            Span::raw(" Tab: "),
            choice(format!("squash onto {}", view.base), squash),
            Span::raw("  "),
            choice("push + PR".into(), !squash),
        ]),
        Line::default(),
        Line::from(match (form.drafting, squash) {
            (true, _) => " The Agent is drafting…",
            (false, true) => " Commit message:",
            (false, false) => " PR title, then body:",
        })
        .dark_gray(),
    ];
    let text = format!("{}▏", form.message);
    lines.extend(text.split('\n').map(|line| Line::from(format!(" {line}"))));
    lines.push(Line::default());
    lines.push(Line::from(" Enter land · Ctrl+g $EDITOR · Esc cancel").dark_gray());
    show(frame, " :land ", Color::Green, lines, 76);
}

fn discard(frame: &mut Frame, view: &SessionView, preview: &DiscardConfirm) {
    let mut lines = vec![
        Line::from(format!(
            " Discard {} / {}?",
            repo_name(&view.repo),
            view.slug
        ))
        .bold(),
        Line::default(),
    ];
    if preview.uncommitted.is_empty() && preview.unlanded.is_empty() {
        lines.push(Line::from(" nothing uncommitted and no unlanded commits").dark_gray());
    } else {
        lines.push(Line::from(" You will lose:").red());
        lines.extend(
            preview
                .uncommitted
                .iter()
                .map(|file| Line::from(format!("   {file} (uncommitted)"))),
        );
        lines.extend(
            preview
                .unlanded
                .iter()
                .map(|commit| Line::from(format!("   {commit}"))),
        );
    }
    lines.push(Line::default());
    lines.push(Line::from(" y discard · any other key cancels").dark_gray());
    show(frame, " :discard ", Color::Red, lines, 76);
}

pub(super) fn mismatch(frame: &mut Frame, message: &str) {
    let lines = vec![
        Line::from(format!(" {message}")),
        Line::default(),
        Line::from(" Live Sessions survive a Daemon restart.").dark_gray(),
        Line::from(" r restart the Daemon · q quit").bold(),
    ];
    show(frame, " Daemon version mismatch ", Color::Yellow, lines, 90);
}

fn trust(frame: &mut Frame, repo: &std::path::Path, items: &[String]) {
    let mut lines = vec![
        Line::from(format!(
            " {} brings scripts or Presets that need your Trust:",
            repo.display()
        )),
        Line::default(),
    ];
    lines.extend(
        items
            .iter()
            .map(|item| Line::from(format!("   {item}")).yellow()),
    );
    lines.push(Line::default());
    lines.push(Line::from(" y trust and create · n cancel").dark_gray());
    show(frame, " Trust ", Color::Yellow, lines, 84);
}

fn guard(frame: &mut Frame, prompt: &GuardPrompt) {
    let lines = vec![
        Line::from(vec![
            Span::raw(" The Agent wants to use "),
            Span::raw(prompt.tool.clone()).bold(),
            Span::raw(format!(" on the {}:", guard_kind(prompt.kind))),
        ]),
        Line::default(),
        Line::from(format!("   {}", prompt.target)).yellow(),
        Line::default(),
        Line::from(" 1 allow once · 2 allow for Session · 3 deny · Esc later").dark_gray(),
    ];
    show(frame, " Guard ", Color::Yellow, lines, 70);
}

fn guard_kind(kind: GuardKindView) -> &'static str {
    match kind {
        GuardKindView::BaseBranch => "Base branch",
        GuardKindView::OtherRef => "another ref",
        GuardKindView::WorktreeManagement => "worktrees",
        GuardKindView::WriteOutsideWorktree => "files outside the Worktree",
    }
}

fn show(frame: &mut Frame, title: &str, colour: Color, lines: Vec<Line<'static>>, width: u16) {
    let height = lines.len() as u16 + 2;
    let area = centered(frame.area(), width, height);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(colour))
        .title(title.to_string());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        area,
    );
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
