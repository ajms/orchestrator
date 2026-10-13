use orch_protocol::AgentStateView;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, Focus, Mode};

const BADGE_FROM: f64 = 80.0;
const BADGE_RED_FROM: f64 = 95.0;

pub(super) fn draw(app: &App, frame: &mut Frame, area: Rect) {
    let summary = summary(app);
    let width = summary.width() as u16;
    let [left, right] =
        Layout::horizontal([Constraint::Min(10), Constraint::Length(width)]).areas(area);
    frame.render_widget(Paragraph::new(mode_line(app)), left);
    frame.render_widget(Paragraph::new(summary), right);
}

fn mode_line(app: &App) -> Line<'static> {
    let (label, colour) = match &app.mode {
        Mode::Normal => (" NORMAL ", Color::Blue),
        Mode::Insert => (" INSERT ", Color::Green),
        Mode::CommandLine(_) => (" COMMAND-LINE ", Color::Magenta),
        Mode::Visual(selection) if selection.linewise => (" V-LINE ", Color::Yellow),
        Mode::Visual(_) => (" VISUAL ", Color::Yellow),
    };
    let rest = match (&app.mode, &app.message) {
        (Mode::CommandLine(line), _) => format!(":{line}▏"),
        (_, Some(message)) => message.clone(),
        _ if app.guard_waiting_hidden() => "Guard prompt waiting — :guard to answer".into(),
        (Mode::Insert, None) => {
            "keys go to the Agent · Ctrl-\\ Ctrl-n → Normal · Ctrl-h → sidebar".into()
        }
        (Mode::Visual(_), None) => "h/j/k/l extend · y yank · Esc cancel".into(),
        (Mode::Normal, None) if app.on_subagent() => match app.focus {
            Focus::Sidebar => {
                "Enter/l focus · J/K subagents · o full results · Esc/h back · i insert".into()
            }
            Focus::Pane => {
                "j/k Ctrl-d/u gg/G scroll · o full results · Esc/h back · i insert".into()
            }
        },
        (Mode::Normal, None) => {
            "j/k select · i insert · za fold · Ctrl-w h/l focus · d review · : commands".into()
        }
    };
    Line::from(vec![
        Span::styled(
            label,
            Style::new()
                .fg(Color::Black)
                .bg(colour)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" {rest}")),
    ])
}

fn summary(app: &App) -> Line<'static> {
    let counts = [
        (AgentStateView::NeedsInput, "Needs input", Color::Yellow),
        (AgentStateView::Working, "Working", Color::Green),
        (AgentStateView::Errored, "Errored", Color::Red),
    ];
    let mut spans = Vec::new();
    for (state, label, colour) in counts {
        let count = app.sessions.count(state);
        if count > 0 {
            spans.push(Span::styled(
                format!(" {count} {label} "),
                Style::new().fg(Color::Black).bg(colour),
            ));
            spans.push(Span::raw(" "));
        }
    }
    let findings = app.findings();
    if findings > 0 {
        spans.push(Span::styled(
            format!("⚠ {findings} findings "),
            Style::new().fg(Color::Yellow),
        ));
    }
    let unseen = app.sessions.unseen();
    if unseen > 0 {
        spans.push(Span::styled(
            format!("● {unseen} "),
            Style::new().fg(Color::Yellow),
        ));
    }
    let limits = [
        ("5h", app.rate_limits.five_hour),
        ("7d", app.rate_limits.seven_day),
    ];
    for (window, used) in limits {
        let Some(used) = used.filter(|used| *used > BADGE_FROM) else {
            continue;
        };
        let colour = match used >= BADGE_RED_FROM {
            true => Color::Red,
            false => Color::Yellow,
        };
        spans.push(Span::styled(
            format!("{window} {used:.0}%"),
            Style::new().fg(colour).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    }
    Line::from(spans)
}
