use crossterm::event::{MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use vt100::MouseProtocolMode;

use crate::app::{App, Call};
use crate::passthrough;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Region {
    Sidebar,
    Pane,
    PaneBorder,
    Statusline,
    Review,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PanePoint {
    pub col: u16,
    pub row: u16,
}

pub(crate) fn handle(app: &mut App, event: MouseEvent) {
    if overlaid(app) {
        app.gesture = None;
        return;
    }
    match owner(app, event) {
        Region::Pane => pane(app, event),
        Region::Sidebar => sidebar(app, event),
        Region::Review => review(app, event),
        Region::PaneBorder | Region::Statusline => {}
    }
}

fn overlaid(app: &App) -> bool {
    app.mismatch.is_some()
        || app.popup.is_some()
        || app.reconcile.is_some()
        || app.guard_prompt().is_some()
}

fn owner(app: &mut App, event: MouseEvent) -> Region {
    let under = region_at(app, event.column, event.row);
    match event.kind {
        MouseEventKind::Down(_) => *app.gesture.insert(under),
        MouseEventKind::Drag(_) => app.gesture.unwrap_or(under),
        MouseEventKind::Up(_) => app.gesture.take().unwrap_or(under),
        _ => under,
    }
}

fn region_at(app: &App, column: u16, row: u16) -> Region {
    let areas = app.areas();
    let at = Position::new(column, row);
    if app.review.is_some() && areas.sidebar.union(areas.pane).contains(at) {
        return Region::Review;
    }
    [
        (areas.pane_body(), Region::Pane),
        (areas.pane, Region::PaneBorder),
        (areas.sidebar, Region::Sidebar),
    ]
    .into_iter()
    .find(|(rect, _)| rect.contains(at))
    .map_or(Region::Statusline, |(_, region)| region)
}

fn pane(app: &mut App, event: MouseEvent) {
    let point = clamped(app.areas().pane_body(), event);
    let requested = app
        .shown_pane()
        .map(|pane| (pane.mouse_mode(), pane.mouse_encoding()))
        .filter(|(mode, _)| *mode != MouseProtocolMode::None);
    match requested {
        Some((mode, encoding)) => {
            if let Some(bytes) = passthrough::encode(event, point, mode, encoding) {
                app.push(Call::Input(bytes));
            }
        }
        None => {
            pane_links(app, event, point);
            pane_selection(app, event, point);
        }
    }
}

fn clamped(body: Rect, event: MouseEvent) -> PanePoint {
    let last_col = body.x + body.width.saturating_sub(1);
    let last_row = body.y + body.height.saturating_sub(1);
    PanePoint {
        col: event.column.clamp(body.x, last_col) - body.x,
        row: event.row.clamp(body.y, last_row) - body.y,
    }
}

fn pane_selection(_app: &mut App, _event: MouseEvent, _point: PanePoint) {}

fn pane_links(_app: &mut App, _event: MouseEvent, _point: PanePoint) {}

fn sidebar(_app: &mut App, _event: MouseEvent) {}

fn review(_app: &mut App, _event: MouseEvent) {}
