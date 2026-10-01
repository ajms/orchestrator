use std::path::Path;
use std::time::Instant;

use orch_protocol::{AgentStateView, PrChecksView, PrReviewView, SessionView, SubagentView};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use super::style;
use crate::app::{App, Focus};
use crate::preparing::{Preparing, PreparingState};
use crate::sessions::repo_label;
use crate::sidebar::{FLAG_INDENT, FLAG_SEPARATOR, Flag, Folded, Row, Urgency};

const LABEL_WIDTH: usize = 20;
const SELECTED: Color = Color::Rgb(50, 50, 70);
const UNSEEN: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);

pub(super) fn draw(app: &App, frame: &mut Frame, area: Rect) {
    let width = usize::from(area.width.saturating_sub(2));
    let rows = app.sidebar_rows();
    let offset = app.sidebar.offset(app.sidebar_viewport(rows.len()));
    let lines: Vec<Line> = rows.iter().map(|row| line(app, row, width)).collect();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(super::border(app.focus == Focus::Sidebar, false))
        .title(" Sessions ");
    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((u16::try_from(offset).unwrap_or(u16::MAX), 0));
    frame.render_widget(paragraph, area);
}

fn line(app: &App, row: &Row, width: usize) -> Line<'static> {
    let selected = row.stop().is_some_and(|stop| app.cursor() == Some(stop));
    match row {
        Row::Blank => Line::default(),
        Row::MissingRepo(repo) => heading(repo, true),
        Row::Heading {
            repo,
            missing,
            folded: None,
        } => heading(repo, *missing),
        Row::Heading {
            repo,
            missing,
            folded: Some(folded),
        } => highlighted(folded_heading(repo, *missing, folded), selected),
        Row::Session(view) => highlighted(first_line(view), selected),
        Row::Flags(_, flags) => flag_line(flags),
        Row::Subagent(_, subagent) => subagent_line(subagent, width),
        Row::SubagentsDone(_, done) => Line::from(format!("    ↳ {done} done")).dark_gray(),
        Row::Preparing(preparing) => {
            highlighted(preparing_line(preparing, (app.clock)(), width), selected)
        }
    }
}

fn highlighted(line: Line<'static>, selected: bool) -> Line<'static> {
    match selected {
        true => line.style(Style::new().bg(SELECTED)),
        false => line,
    }
}

fn heading(repo: &Path, missing: bool) -> Line<'static> {
    Line::from(format!(" {}", repo_label(repo, missing)))
        .bold()
        .underlined()
}

fn folded_heading(repo: &Path, missing: bool, folded: &Folded) -> Line<'static> {
    let mut spans = vec![
        Span::raw(format!(" ▸ {}", repo_label(repo, missing)))
            .bold()
            .underlined(),
        Span::raw(format!(" ({})", folded.count)),
    ];
    spans.extend(folded.urgency.map(urgency));
    Line::from(spans)
}

fn urgency(urgency: Urgency) -> Span<'static> {
    match urgency {
        Urgency::NeedsInput => {
            let (label, colour) = style::agent_state(AgentStateView::NeedsInput);
            Span::styled(format!(" {label}"), Style::new().fg(colour))
        }
        Urgency::Unseen => Span::styled(" ●", UNSEEN),
    }
}

fn first_line(view: &SessionView) -> Line<'static> {
    let marker = match view.flags.unseen {
        true => Span::styled("● ", UNSEEN),
        false => Span::raw("  "),
    };
    let (label, colour) = style::status(view);
    let (name, colour) = match view.flags.repo_missing {
        true => (Style::new().fg(Color::DarkGray), Color::DarkGray),
        false => (Style::new(), colour),
    };
    let mut first = vec![
        marker,
        Span::styled(
            format!(
                "{:<LABEL_WIDTH$} ",
                truncate(view.title_or_slug(), LABEL_WIDTH)
            ),
            name,
        ),
        Span::styled(label, Style::new().fg(colour)),
    ];
    if let Some(percent) = view.context_used_percent {
        first.push(Span::raw(" "));
        first.push(Span::styled(
            format!("{percent:.0}%"),
            Style::new().fg(style::context(percent)),
        ));
    }
    Line::from(first)
}

fn preparing_line(preparing: &Preparing, now: Instant, width: usize) -> Line<'static> {
    let seconds = now.saturating_duration_since(preparing.since).as_secs();
    let room = width.saturating_sub(LABEL_WIDTH + 5);
    let state = match &preparing.state {
        PreparingState::Failed(reason) => Span::styled(
            format!("✗ {}", truncate(reason, room)),
            Style::new().fg(Color::Red),
        ),
        PreparingState::NeedsTrust => Span::styled("needs Trust", Style::new().fg(Color::Yellow)),
        PreparingState::Waiting => Span::styled(
            format!("Preparing… {seconds}s"),
            Style::new().fg(Color::Cyan),
        ),
    };
    Line::from(vec![
        Span::raw(format!(
            "  {:<LABEL_WIDTH$} ",
            truncate(&preparing.slug, LABEL_WIDTH)
        )),
        state,
    ])
}

fn subagent_line(subagent: &SubagentView, width: usize) -> Line<'static> {
    let text = format!(
        "{}: {} · {} tools",
        subagent.agent_type, subagent.description, subagent.tool_count
    );
    Line::from(vec![
        Span::raw("    ↳ ").dark_gray(),
        Span::raw(truncate(&text, width.saturating_sub(6).max(1))),
    ])
}

fn flag_line(flags: &[Flag]) -> Line<'static> {
    let mut spans = vec![Span::raw(" ".repeat(FLAG_INDENT))];
    for (at, flag) in flags.iter().enumerate() {
        if at > 0 {
            spans.push(Span::raw(FLAG_SEPARATOR).dark_gray());
        }
        spans.push(Span::styled(flag.text(), flag_style(*flag)));
    }
    Line::from(spans)
}

fn flag_style(flag: Flag) -> Style {
    let colour = match flag {
        Flag::Pr(_) => Color::Magenta,
        Flag::Checks(PrChecksView::Pending) => Color::Yellow,
        Flag::Checks(PrChecksView::Passing) => Color::Green,
        Flag::Checks(_) => Color::Red,
        Flag::Review(PrReviewView::Approved) => Color::Green,
        Flag::Review(PrReviewView::ChangesRequested) => Color::Yellow,
        Flag::Review(_) | Flag::NewComments(_) => return Style::new(),
        Flag::PrClosed => Color::Red,
        Flag::NeedsRebase => Color::Yellow,
        Flag::Stalled => Color::Red,
        Flag::Recovered => Color::Cyan,
        Flag::WorktreeMissing | Flag::BaseMissing => Color::Red,
        Flag::Muted => Color::DarkGray,
    };
    Style::new().fg(colour)
}

fn truncate(text: &str, width: usize) -> String {
    match text.chars().count() > width {
        true => text.chars().take(width - 1).chain(['…']).collect(),
        false => text.to_string(),
    }
}
