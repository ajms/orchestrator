use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use crate::layout::Columns;
use crate::review::ReviewView;

const SELECTED: Color = Color::Rgb(50, 50, 70);

pub(super) fn draw(review: &ReviewView, frame: &mut Frame, area: Rect) {
    let columns = Columns::of(area);
    let rows: Vec<Line> = review
        .files
        .iter()
        .enumerate()
        .map(|(at, file)| {
            let line = Line::from(vec![
                Span::raw(format!(" {} ", file.path)),
                Span::styled(format!("+{}", file.added()), Style::new().fg(Color::Green)),
                Span::raw(" "),
                Span::styled(format!("-{}", file.removed()), Style::new().fg(Color::Red)),
            ]);
            match at == review.file {
                true => line.style(Style::new().bg(SELECTED)),
                false => line,
            }
        })
        .collect();
    let rows = match rows.is_empty() {
        true => vec![Line::from(format!(" no changes against {}", review.base))],
        false => rows,
    };
    frame.render_widget(
        Paragraph::new(rows).scroll((review.list_scroll, 0)).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(format!(" Review vs {} ", review.base)),
        ),
        columns.files,
    );
    let body: Vec<Line> = review
        .files
        .get(review.file)
        .map(|file| file.lines.iter().map(|line| diff_line(line)).collect())
        .unwrap_or_default();
    frame.render_widget(
        Paragraph::new(body).scroll((review.scroll, 0)).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" j/k file · Ctrl-d/u scroll · o edit · :land · q back "),
        ),
        columns.diff,
    );
    if let Some(span) = review.selection.span() {
        let top = i64::from(review.scroll);
        super::pane::paint_selection(frame, columns.diff_body(), top, span, false);
    }
}

fn diff_line(line: &str) -> Line<'static> {
    let colour = match line.chars().next() {
        Some('+') => Color::Green,
        Some('-') => Color::Red,
        Some('@') => Color::Cyan,
        _ => Color::Reset,
    };
    Line::styled(line.to_string(), Style::new().fg(colour))
}
