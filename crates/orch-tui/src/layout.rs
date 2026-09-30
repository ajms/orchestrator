use orch_protocol::Size;
use ratatui::layout::{Constraint, Layout, Margin, Rect};

pub const SIDEBAR_WIDTH: u16 = 40;
const FILE_LIST_WIDTH: u16 = 40;

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

    pub fn main(&self) -> Rect {
        self.sidebar.union(self.pane)
    }

    pub fn pane_body(&self) -> Rect {
        self.pane.inner(Margin::new(1, 1))
    }

    pub fn pane_inner(&self) -> Size {
        let body = self.pane_body();
        Size {
            rows: body.height.max(1),
            cols: body.width.max(1),
        }
    }
}

pub struct Columns {
    pub files: Rect,
    pub diff: Rect,
}

impl Columns {
    pub fn of(main: Rect) -> Self {
        let [files, diff] =
            Layout::horizontal([Constraint::Length(FILE_LIST_WIDTH), Constraint::Min(20)])
                .areas(main);
        Self { files, diff }
    }

    pub fn files_body(&self) -> Rect {
        self.files.inner(Margin::new(1, 1))
    }

    pub fn diff_body(&self) -> Rect {
        self.diff.inner(Margin::new(1, 1))
    }
}
