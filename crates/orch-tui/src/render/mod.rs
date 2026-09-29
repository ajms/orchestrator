mod pane;
mod popup;
mod reconcile;
mod review;
mod sidebar;
mod statusline;
mod style;

use ratatui::Frame;
use ratatui::style::{Color, Style};

use crate::app::App;
use crate::layout::Areas;

pub(crate) fn draw(app: &App, frame: &mut Frame) {
    if let Some(message) = &app.mismatch {
        return popup::mismatch(frame, message);
    }
    let areas = Areas::of(frame.area());
    let main = areas.sidebar.union(areas.pane);
    match (&app.reconcile, &app.review) {
        (Some(open), _) => reconcile::draw(open, frame, main),
        (None, Some(open)) => review::draw(open, frame, main),
        (None, None) => {
            sidebar::draw(app, frame, areas.sidebar);
            pane::draw(app, frame, areas.pane);
        }
    }
    statusline::draw(app, frame, areas.statusline);
    popup::draw(app, frame);
}

fn border(focused: bool, inserting: bool) -> Style {
    match (focused, inserting) {
        (true, true) => Style::new().fg(Color::Green),
        (true, false) => Style::new().fg(Color::Cyan),
        _ => Style::new().fg(Color::DarkGray),
    }
}
