use orch_protocol::{PrChecksView, PrReviewView, SessionView};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use super::style;
use crate::app::{App, Focus};
use crate::sessions::repo_name;

const SLUG_WIDTH: usize = 20;
const SELECTED: Color = Color::Rgb(50, 50, 70);

pub(super) fn draw(app: &App, frame: &mut Frame, area: Rect) {
    let width = usize::from(area.width.saturating_sub(2));
    let mut lines = Vec::new();
    for repo in app.sessions.repos() {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        lines.push(
            Line::from(format!(" {}", repo_name(repo)))
                .bold()
                .underlined(),
        );
        for view in app.sessions.in_repo(repo) {
            let selected = app.selected.as_ref() == Some(&view.id);
            lines.extend(session_lines(view, width, selected));
        }
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(super::border(app.focus == Focus::Sidebar, false))
        .title(" Sessions ");
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn session_lines(view: &SessionView, width: usize, selected: bool) -> Vec<Line<'static>> {
    let marker = match view.flags.unseen {
        true => Span::styled("● ", Style::new().fg(Color::Yellow).bold()),
        false => Span::raw("  "),
    };
    let (label, colour) = style::status(view);
    let mut first = vec![
        marker,
        Span::raw(format!(
            "{:<SLUG_WIDTH$} ",
            truncate(&view.slug, SLUG_WIDTH)
        )),
        Span::styled(label, Style::new().fg(colour)),
    ];
    if let Some(percent) = view.context_used_percent {
        first.push(Span::raw(" "));
        first.push(Span::styled(
            format!("{percent:.0}%"),
            Style::new().fg(style::context(percent)),
        ));
    }
    let mut first = Line::from(first);
    if selected {
        first = first.style(Style::new().bg(SELECTED));
    }
    let mut lines = vec![first];
    lines.extend(wrap(flags(view), width));
    lines.extend(subagent_lines(view, width));
    lines
}

fn subagent_lines(view: &SessionView, width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = view
        .subagents
        .iter()
        .filter(|subagent| !subagent.done)
        .map(|subagent| {
            let text = format!(
                "{}: {} · {} tools",
                subagent.agent_type, subagent.description, subagent.tool_count
            );
            Line::from(vec![
                Span::raw("    ↳ ").dark_gray(),
                Span::raw(truncate(&text, width.saturating_sub(6).max(1))),
            ])
        })
        .collect();
    let done = view
        .subagents
        .iter()
        .filter(|subagent| subagent.done)
        .count();
    if done > 0 {
        lines.push(Line::from(format!("    ↳ {done} done")).dark_gray());
    }
    lines
}

fn flags(view: &SessionView) -> Vec<Span<'static>> {
    let flags = &view.flags;
    let mut spans = Vec::new();
    if let Some(number) = flags.pr_number {
        spans.push(Span::styled(
            format!("PR #{number}"),
            Style::new().fg(Color::Magenta),
        ));
    }
    if let Some(pr) = &flags.pr {
        match pr.checks {
            PrChecksView::None => {}
            PrChecksView::Pending => {
                spans.push(Span::styled("⋯ checks", Style::new().fg(Color::Yellow)))
            }
            PrChecksView::Passing => {
                spans.push(Span::styled("✓ checks", Style::new().fg(Color::Green)))
            }
            PrChecksView::Failing => {
                spans.push(Span::styled("✗ checks", Style::new().fg(Color::Red)))
            }
        }
        match pr.review {
            PrReviewView::None => {}
            PrReviewView::ReviewRequired => spans.push(Span::raw("review required")),
            PrReviewView::Approved => {
                spans.push(Span::styled("approved", Style::new().fg(Color::Green)))
            }
            PrReviewView::ChangesRequested => spans.push(Span::styled(
                "changes requested",
                Style::new().fg(Color::Yellow),
            )),
        }
        if pr.new_comments > 0 {
            spans.push(Span::raw(format!("{} new", pr.new_comments)));
        }
        if pr.closed {
            spans.push(Span::styled(
                "PR closed (:abandon)",
                Style::new().fg(Color::Red),
            ));
        }
    }
    let marks = [
        (flags.needs_rebase, "⟲ rebase", Color::Yellow),
        (flags.stalled, "stalled?", Color::Red),
        (flags.recovered, "recovered", Color::Cyan),
        (flags.worktree_missing, "worktree missing", Color::Red),
        (flags.base_missing, "base missing", Color::Red),
        (flags.muted, "muted", Color::DarkGray),
    ];
    spans.extend(
        marks
            .into_iter()
            .filter(|(set, _, _)| *set)
            .map(|(_, text, colour)| Span::styled(text, Style::new().fg(colour))),
    );
    spans
}

fn wrap(items: Vec<Span<'static>>, width: usize) -> Vec<Line<'static>> {
    const INDENT: &str = "    ";
    let mut lines = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for item in items {
        let len = item.content.chars().count();
        if !current.is_empty() && used + 3 + len > width {
            lines.push(Line::from(std::mem::take(&mut current)));
        }
        if current.is_empty() {
            current.push(Span::raw(INDENT));
            used = INDENT.len();
        } else {
            current.push(Span::raw(" · ").dark_gray());
            used += 3;
        }
        used += len;
        current.push(item);
    }
    if !current.is_empty() {
        lines.push(Line::from(current));
    }
    lines
}

fn truncate(text: &str, width: usize) -> String {
    match text.chars().count() > width {
        true => text.chars().take(width - 1).chain(['…']).collect(),
        false => text.to_string(),
    }
}
