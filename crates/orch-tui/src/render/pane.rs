use orch_protocol::{PhaseView, SessionView};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph, Wrap};
use tui_term::widget::PseudoTerminal;

use super::style;
use crate::app::{App, Focus, Mode, Selection};
use crate::pane::PaneMirror;

const SELECTION: Color = Color::Rgb(70, 70, 110);
use crate::sessions::repo_name;

pub(super) fn draw(app: &App, frame: &mut Frame, area: Rect) {
    let focused = app.focus == Focus::Pane;
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(super::border(focused, app.inserting()));
    let Some(view) = app.selected_view() else {
        frame.render_widget(
            Paragraph::new(" No Sessions yet — :new starts one").block(block),
            area,
        );
        return;
    };
    block = block.title(title(view));
    match &app.pane {
        Some(pane) if pane.session == view.id && pane.closed.is_none() => {
            let inner = block.inner(area);
            frame.render_widget(PseudoTerminal::new(pane.screen()).block(block), area);
            if let Mode::Visual(selection) = &app.mode {
                highlight(frame, inner, selection, pane);
            }
        }
        pane => {
            let closed = pane
                .as_ref()
                .filter(|pane| pane.session == view.id)
                .and_then(|pane| pane.closed.clone());
            let text = placeholder(view, closed);
            frame.render_widget(
                Paragraph::new(text).wrap(Wrap { trim: false }).block(block),
                area,
            );
        }
    }
}

fn highlight(frame: &mut Frame, inner: Rect, selection: &Selection, pane: &PaneMirror) {
    let (start, end) = selection.ordered();
    let buffer = frame.buffer_mut();
    let first = pane.row_of(start.0).max(0);
    let last = pane.row_of(end.0).min(i64::from(inner.height) - 1);
    for row in first..=last {
        let line = pane.line_of(row as u16);
        let row = row as u16;
        let (from, to) = match selection.linewise {
            true => (0, inner.width.saturating_sub(1)),
            false => (
                if line == start.0 { start.1 } else { 0 },
                if line == end.0 {
                    end.1
                } else {
                    inner.width.saturating_sub(1)
                },
            ),
        };
        for col in from..=to.min(inner.width.saturating_sub(1)) {
            buffer[(inner.x + col, inner.y + row)].set_bg(SELECTION);
        }
    }
}

fn title(view: &SessionView) -> Line<'static> {
    let (label, colour) = style::status(view);
    let mode = match &view.mode {
        Some(mode) => format!("[{} · {mode}]", view.preset),
        None => format!("[{}]", view.preset),
    };
    Line::from(vec![
        Span::raw(" "),
        Span::raw(format!("{} / {} ", repo_name(&view.repo), view.slug)).bold(),
        Span::styled(label, Style::new().fg(colour)),
        Span::raw(format!(" {mode} ")),
    ])
}

fn placeholder(view: &SessionView, closed: Option<String>) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();
    if let Some(output) = &view.setup_output {
        lines.extend(output.lines().map(|line| Line::from(line.to_string())));
        lines.push(Line::default());
    }
    if let Some(error) = &view.error {
        lines.push(Line::from(error.clone()).red());
    }
    if let Some(reason) = closed {
        lines.push(Line::from(format!("Pane closed: {reason}")).dark_gray());
    }
    let hint = match view.phase {
        PhaseView::SettingUp => "Running the Setup script…",
        PhaseView::SetupFailed => "Setup failed — :retry · :start (anyway) · :discard",
        PhaseView::Suspended => "Suspended — i / :resume continues the Conversation",
        PhaseView::Landed => "Landed",
        PhaseView::Discarded => "Discarded",
        PhaseView::Active | PhaseView::PrOpen => "i / :resume restarts the Agent",
    };
    lines.push(Line::from(hint).yellow());
    lines
}
