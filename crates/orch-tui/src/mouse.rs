use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use vt100::MouseProtocolMode;

use crate::app::{App, Call, Mode};
use crate::hyperlinks::{Hyperlink, HyperlinkTarget, hyperlink_at};
use crate::layout::Columns;
use crate::passthrough;
use crate::review::{EditorTarget, ReviewAction, ReviewView};

const WHEEL_LINES: isize = 3;

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
    app.links.hover(None);
    if overlaid(app) {
        app.end_gesture();
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
            if !pane_links(app, event, point) {
                pane_selection(app, event, point);
            }
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

fn pane_selection(app: &mut App, event: MouseEvent, point: PanePoint) {
    let row = body_row(app.areas().pane_body(), event);
    let now = (app.clock)();
    if app.shown_pane().is_none() {
        return;
    }
    let (Some(pane), selector) = (app.pane.as_mut(), &mut app.pane_selection) else {
        return;
    };
    match event.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if matches!(app.mode, Mode::Visual(_)) {
                app.mode = Mode::Normal;
            }
            selector.press(pane, point.col, row, now);
        }
        MouseEventKind::Drag(MouseButton::Left) if app.gesture.is_some() => {
            selector.extend(pane, point.col, row)
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if let Some(text) = selector.release(pane) {
                app.copy(&text);
            }
        }
        MouseEventKind::ScrollUp => pane.scroll_by(WHEEL_LINES),
        MouseEventKind::ScrollDown => pane.scroll_by(-WHEEL_LINES),
        _ => {}
    }
}

pub(crate) fn auto_scrolling(app: &App) -> bool {
    match app.gesture {
        Some(Region::Pane) => app.pane_selection.auto_scrolling(),
        Some(Region::Review) => app
            .review
            .as_ref()
            .is_some_and(|review| review.selection.auto_scrolling()),
        _ => false,
    }
}

pub(crate) fn tick(app: &mut App) {
    if !auto_scrolling(app) {
        return;
    }
    let body = Columns::of(app.areas().main()).diff_body();
    if app.gesture == Some(Region::Review)
        && let Some(review) = app.review.as_mut()
    {
        review.tick(body);
    } else if let Some(pane) = app.pane.as_mut() {
        app.pane_selection.tick(pane);
    }
}

fn pane_links(app: &mut App, event: MouseEvent, point: PanePoint) -> bool {
    let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
    match event.kind {
        MouseEventKind::Down(button) => {
            app.links.let_go();
            if !ctrl || button != MouseButton::Left {
                return false;
            }
            let Some(link) = link_under(app, point) else {
                return false;
            };
            app.links.press();
            open(app, link.target);
            true
        }
        MouseEventKind::Drag(_) => app.links.pressed(),
        MouseEventKind::Up(_) => {
            let pressed = app.links.pressed();
            app.links.let_go();
            pressed
        }
        MouseEventKind::Moved if ctrl => {
            let link = link_under(app, point);
            app.links.hover(link);
            true
        }
        _ => false,
    }
}

fn link_under(app: &mut App, point: PanePoint) -> Option<Hyperlink> {
    let worktree = app.selected_view()?.worktree.clone();
    let pane = app.pane.as_mut()?;
    let line = pane.line_of(point.row);
    hyperlink_at(pane, (line, point.col), &worktree)
}

fn open(app: &mut App, target: HyperlinkTarget) {
    match target {
        HyperlinkTarget::Url(url) => app.open_url(url),
        HyperlinkTarget::File { path, line } => app.edit(EditorTarget { file: path, line }),
    }
}

fn sidebar(_app: &mut App, _event: MouseEvent) {}

fn review(app: &mut App, event: MouseEvent) {
    let columns = Columns::of(app.areas().main());
    let (list, body) = (columns.files_body(), columns.diff_body());
    let at = Position::new(event.column, event.row);
    let ctrl = event.modifiers.contains(KeyModifiers::CONTROL);
    let now = (app.clock)();
    let Some(review) = app.review.as_mut() else {
        return;
    };
    let action = match event.kind {
        MouseEventKind::Down(MouseButton::Left) if list.contains(at) => {
            if let Some(file) = review.file_at(event.row - list.y) {
                review.show_file(file);
            }
            ReviewAction::Stay
        }
        MouseEventKind::Down(MouseButton::Left) if body.contains(at) && ctrl => {
            review.open_row(body, event.row - body.y)
        }
        MouseEventKind::Down(MouseButton::Left) if body.contains(at) => {
            review.press(body, clamped(body, event).col, body_row(body, event), now);
            ReviewAction::Stay
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            review.extend(body, clamped(body, event).col, body_row(body, event));
            ReviewAction::Stay
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if let Some(text) = review.release(body) {
                app.copy(&text);
            }
            ReviewAction::Stay
        }
        MouseEventKind::ScrollUp => {
            review_wheel(review, (list, body), at, WHEEL_LINES);
            ReviewAction::Stay
        }
        MouseEventKind::ScrollDown => {
            review_wheel(review, (list, body), at, -WHEEL_LINES);
            ReviewAction::Stay
        }
        _ => ReviewAction::Stay,
    };
    app.review_action(action);
}

fn review_wheel(review: &mut ReviewView, (list, body): (Rect, Rect), at: Position, lines: isize) {
    if list.contains(at) {
        review.scroll_list(lines, list.height);
    } else if body.contains(at) {
        review.scroll_diff(lines, body);
    }
}

fn body_row(body: Rect, event: MouseEvent) -> i32 {
    i32::from(event.row) - i32::from(body.y)
}
