use orch_protocol::Size;
use ratatui::layout::{Constraint, Layout, Rect};

pub const SIDEBAR_WIDTH: u16 = 40;

pub struct Areas {
    pub sidebar: Rect,
    pub pane: Rect,
    pub statusline: Rect,
}

impl Areas {
    pub fn of(area: Rect) -> Self {
        let [main, statusline] =
            Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
        let [sidebar, pane] =
            Layout::horizontal([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(10)])
                .areas(main);
        Self {
            sidebar,
            pane,
            statusline,
        }
    }

    pub fn for_size(size: Size) -> Self {
        Self::of(Rect::new(0, 0, size.cols, size.rows))
    }

    pub fn pane_inner(&self) -> Size {
        Size {
            rows: self.pane.height.saturating_sub(2).max(1),
            cols: self.pane.width.saturating_sub(2).max(1),
        }
    }
}
