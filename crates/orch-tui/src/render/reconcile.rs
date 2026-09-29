use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Paragraph};

use crate::reconcile::{ReconcileView, Row};

const SELECTED: Color = Color::Rgb(50, 50, 70);

pub(super) fn draw(view: &ReconcileView, frame: &mut Frame, area: Rect) {
    let mut fix_index = 0;
    let mut lines: Vec<Line> = Vec::new();
    for row in &view.rows {
        match row {
            Row::Heading(name) => {
                if !lines.is_empty() {
                    lines.push(Line::default());
                }
                lines.push(Line::from(format!(" {name}")).bold().underlined());
            }
            Row::Finding(text) => lines.push(Line::from(format!("   {text}")).yellow()),
            Row::Fix(fix) => {
                let line = Line::from(format!("     → {}", fix.label()));
                lines.push(match fix_index == view.selected {
                    true => line.style(Style::new().bg(SELECTED)),
                    false => line,
                });
                fix_index += 1;
            }
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(" Everything matches: no findings.").dark_gray());
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(" Reconciliation — j/k fix · Enter apply · q back ");
    frame.render_widget(Paragraph::new(lines).block(block), area);
}
