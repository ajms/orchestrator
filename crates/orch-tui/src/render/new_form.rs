use std::path::Path;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::new_form::{Field, NewForm};
use crate::repo_picker::{PickerRow, RepoPicker};
use crate::sessions::repo_name;
use crate::text_input::TextInput;

const STACKED_BELOW: u16 = 100;
const FIELDS_WIDTH: u16 = 40;
const ROW_LABEL: usize = 7;
const FORM_FIELDS: [Field; 5] = [
    Field::Repo,
    Field::Branch,
    Field::Base,
    Field::Agent,
    Field::Preset,
];

pub(super) fn draw(frame: &mut Frame, form: &NewForm, home: Option<&Path>) {
    let screen = frame.area();
    let width = screen.width.saturating_sub(6).min(130);
    let height = screen.height.saturating_sub(4).min(26);
    let area = super::popup::centered(screen, width, height);
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::Cyan))
        .title(" :new Session ")
        .title_bottom(hints(form));
    let body = block.inner(area);
    frame.render_widget(block, area);
    let cursor = match screen.width < STACKED_BELOW {
        true => stacked(frame, form, body, home),
        false => columns(frame, form, body, home),
    };
    if let Some(cursor) = cursor {
        frame.set_cursor_position(cursor);
    }
}

fn columns(frame: &mut Frame, form: &NewForm, body: Rect, home: Option<&Path>) -> Option<Position> {
    let left_width = match form.repo.picker() {
        Some(_) => body.width * 3 / 5,
        None => FIELDS_WIDTH,
    };
    let [left, right] =
        Layout::horizontal([Constraint::Length(left_width), Constraint::Min(10)]).areas(body);
    let divider = Block::new()
        .borders(Borders::RIGHT)
        .border_style(Style::new().fg(Color::DarkGray));
    let fields = divider.inner(left);
    frame.render_widget(divider, left);
    let cursor = match form.repo.picker() {
        Some(picker) => self::picker(frame, picker, fields),
        None => {
            let mut lines = Vec::new();
            let mut cursor = None;
            for field in FORM_FIELDS {
                lines.push(Line::from(label(
                    form,
                    field,
                    usize::from(fields.width) - 2,
                )));
                let at = Rect {
                    y: fields.y + lines.len() as u16,
                    ..fields
                };
                let (value, at) = value(form, field, at, home);
                cursor = cursor.or(at);
                lines.push(value);
                lines.push(Line::default());
            }
            lines.extend(error(form));
            frame.render_widget(Paragraph::new(lines), fields);
            cursor
        }
    };
    let [head, text] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(right);
    frame.render_widget(Paragraph::new(label(form, Field::Prompt, 6)), head);
    prompt(frame, form, text.inner(Margin::new(1, 0))).or(cursor)
}

fn stacked(frame: &mut Frame, form: &NewForm, body: Rect, home: Option<&Path>) -> Option<Position> {
    let rows = match form.repo.picker() {
        Some(_) => body.height / 2,
        None => FORM_FIELDS.len() as u16 + u16::from(form.error.is_some()) + 1,
    };
    let [fields, head, text] = Layout::vertical([
        Constraint::Length(rows),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas(body);
    let cursor = match form.repo.picker() {
        Some(picker) => self::picker(frame, picker, fields),
        None => {
            let mut lines = Vec::new();
            let mut cursor = None;
            for field in FORM_FIELDS {
                let at = Rect {
                    x: fields.x + ROW_LABEL as u16 + 3,
                    y: fields.y + lines.len() as u16,
                    width: fields.width.saturating_sub(ROW_LABEL as u16 + 4),
                    height: 1,
                };
                let (value, at) = value(form, field, at, home);
                cursor = cursor.or(at);
                let mut line = Line::from(vec![label(form, field, ROW_LABEL), Span::raw(" ")]);
                line.spans.extend(value.spans);
                lines.push(line);
            }
            lines.extend(error(form));
            frame.render_widget(Paragraph::new(lines), fields);
            cursor
        }
    };
    frame.render_widget(Paragraph::new(label(form, Field::Prompt, 6)), head);
    prompt(frame, form, text.inner(Margin::new(1, 0))).or(cursor)
}

fn hints(form: &NewForm) -> Line<'static> {
    if form.discarding {
        return Line::from(" Esc again to discard · any other key keeps editing ").yellow();
    }
    let keys = match (form.repo.picker(), form.field) {
        (Some(_), _) => {
            "type to filter, / or ~ for a path · ↑/↓ choose · Tab descend · Enter pick · Esc back"
        }
        (None, Field::Prompt) => {
            "Enter newline · Ctrl+g $EDITOR · Ctrl+r Repo · Ctrl+s create · Esc cancel"
        }
        (None, Field::Repo) => "Enter pick Repo · Tab next · Ctrl+s create · Esc cancel",
        (None, Field::Branch) => "Ctrl+u clear · Enter next · Ctrl+s create · Esc cancel",
        (None, Field::Base) => "↑/↓ other Branches · Enter next · Ctrl+s create · Esc cancel",
        (None, Field::Agent | Field::Preset) => {
            "←/→ choose · Enter next · Ctrl+s create · Esc cancel"
        }
    };
    Line::from(format!(" {keys} ")).dark_gray()
}

fn label(form: &NewForm, field: Field, width: usize) -> Span<'static> {
    let name = match field {
        Field::Repo => "Repo",
        Field::Prompt => "Prompt",
        Field::Branch => "Branch",
        Field::Base => "Base",
        Field::Agent => "Agent",
        Field::Preset => "Preset",
    };
    let style = match form.field == field && form.repo.picker().is_none() {
        true => Style::new().fg(Color::Black).bg(Color::Cyan),
        false => Style::new().fg(Color::DarkGray),
    };
    Span::styled(format!(" {name:<width$} "), style)
}

fn value(
    form: &NewForm,
    field: Field,
    area: Rect,
    home: Option<&Path>,
) -> (Line<'static>, Option<Position>) {
    let focused = form.field == field && form.repo.picker().is_none();
    let line = match field {
        Field::Repo => match form.repo.path() {
            Some(repo) if form.repo.moved() => Line::from(format!(" → {}", tilde(repo, home))),
            Some(repo) => Line::from(format!(" {}", repo.display())),
            None => Line::from(" (pick a Repo)").dark_gray(),
        },
        Field::Branch if form.branch.suggested() => {
            return single_line(&form.branch, focused, area, Style::new().dark_gray());
        }
        Field::Branch => return single_line(&form.branch, focused, area, Style::new()),
        Field::Base if form.base.text().is_empty() => {
            let default = match &form.default_base {
                Some(default) => format!("{default} (Repo default)"),
                None => "(Repo default)".to_string(),
            };
            let cursor = focused.then_some(Position::new(area.x + 1, area.y));
            return (Line::from(format!(" {default}")).dark_gray(), cursor);
        }
        Field::Base => return single_line(&form.base, focused, area, Style::new()),
        Field::Agent => match form.agent.map(|at| &form.agents[at]) {
            Some(agent) if agent.unavailable.is_some() => {
                Line::from(format!(" ◂ {} (unavailable) ▸", agent.name)).red()
            }
            Some(agent) => Line::from(format!(" ◂ {} ▸", agent.name)),
            None => Line::from(" ◂ (Repo default) ▸"),
        },
        Field::Preset => Line::from(match form.preset {
            Some(at) => format!(" ◂ {} ▸", form.presets[at]),
            None => match &form.default_preset {
                Some(default) => format!(" ◂ {default} (Repo default) ▸"),
                None => " ◂ (Repo default) ▸".to_string(),
            },
        }),
        Field::Prompt => Line::default(),
    };
    (line, None)
}

fn single_line(
    input: &TextInput,
    focused: bool,
    area: Rect,
    style: Style,
) -> (Line<'static>, Option<Position>) {
    let text = input.text();
    let room = usize::from(area.width.saturating_sub(2)).max(1);
    let mut start = 0;
    while text[start..input.cursor()].width() > room {
        start += text[start..].chars().next().map_or(1, char::len_utf8);
    }
    let column = text[start..input.cursor()].width() as u16;
    let cursor = focused.then_some(Position::new(area.x + 1 + column, area.y));
    let line = Line::from(Span::styled(format!(" {}", &text[start..]), style));
    (line, cursor)
}

fn tilde(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

fn error(form: &NewForm) -> Option<Line<'static>> {
    form.error
        .as_ref()
        .map(|error| Line::from(format!(" {error}")).red())
}

fn picker(frame: &mut Frame, picker: &RepoPicker, area: Rect) -> Option<Position> {
    let query = picker.query.text();
    let mut lines = vec![Line::from(vec![
        Span::raw(" > ").cyan(),
        Span::raw(query.to_string()),
    ])];
    if let Some(error) = &picker.error {
        lines.push(Line::from(format!(" {error}")).red());
    }
    if picker.rows().is_empty() && !picker.path_mode() {
        lines.push(Line::from(" no matching Repo — start with / or ~ for a path").dark_gray());
    }
    let mut rows: Vec<Line> = picker
        .rows()
        .iter()
        .enumerate()
        .map(|(at, row)| match at == picker.choice {
            true => row_line("▸", row).style(Style::new().bg(Color::Rgb(50, 50, 70))),
            false => row_line(" ", row),
        })
        .collect();
    if picker.unlisted_folders > 0 {
        rows.push(Line::from(format!("   … {} more", picker.unlisted_folders)).dark_gray());
    }
    let room = usize::from(area.height).saturating_sub(lines.len()).max(1);
    picker.page.set(room);
    let last = picker.choice == picker.rows().len().saturating_sub(1);
    let bottom = picker.choice + usize::from(last && picker.unlisted_folders > 0);
    let mut top = picker.top.get().min(picker.choice);
    if bottom >= top + room {
        top = bottom + 1 - room;
    }
    picker.top.set(top);
    lines.extend(rows.into_iter().skip(top).take(room));
    frame.render_widget(Paragraph::new(lines), area);
    let column = query[..picker.query.cursor()].width() as u16;
    Some(Position::new(area.x + 3 + column, area.y))
}

fn row_line(mark: &str, row: &PickerRow) -> Line<'static> {
    const NAME_WIDTH: usize = 22;
    match row {
        PickerRow::Repo(repo) => {
            let mut line = Line::from(vec![
                Span::raw(format!(" {mark} {:<NAME_WIDTH$} ", repo_name(&repo.path))),
                Span::raw(repo.path.display().to_string()).dark_gray(),
            ]);
            if repo.live > 0 {
                line.spans
                    .push(Span::raw(format!("  {} live", repo.live)).dark_gray());
            }
            line
        }
        PickerRow::ThisFolder(path) => Line::from(vec![
            Span::raw(format!(" {mark} this folder  ")),
            Span::raw(path.display().to_string()).dark_gray(),
        ]),
        PickerRow::Folder { path, git } => {
            let mut line = Line::from(format!(" {mark} {}/", repo_name(path)));
            if *git {
                line.spans.push(Span::raw("  git").green());
            }
            line
        }
    }
}

fn prompt(frame: &mut Frame, form: &NewForm, area: Rect) -> Option<Position> {
    let focused = form.field == Field::Prompt && form.repo.picker().is_none();
    let text = form.prompt.text();
    if text.is_empty() && !focused {
        frame.render_widget(
            Paragraph::new(Line::from("What should the Agent do?").dark_gray()),
            area,
        );
        return None;
    }
    let rows = wrap(text, usize::from(area.width.saturating_sub(1)).max(1));
    let cursor = form.prompt.cursor();
    let row = rows
        .iter()
        .rposition(|&(start, _)| start <= cursor)
        .unwrap_or(0);
    let height = usize::from(area.height).max(2);
    let top = scroll(form.prompt_top.get(), row, rows.len(), height);
    form.prompt_top.set(top);
    let room = if top > 0 { height - 1 } else { height };
    let mut lines = Vec::new();
    if top > 0 {
        lines.push(
            Line::from(format!("↑ {top} more lines"))
                .dark_gray()
                .italic(),
        );
    }
    lines.extend(
        rows[top..(top + room).min(rows.len())]
            .iter()
            .map(|&(start, end)| Line::from(text[start..end].to_string())),
    );
    frame.render_widget(Paragraph::new(lines), area);
    let (start, _) = rows[row];
    let x = area.x + text[start..cursor].width() as u16;
    let y = area.y + (row - top + usize::from(top > 0)) as u16;
    focused.then_some(Position::new(x, y))
}

fn scroll(top: usize, row: usize, rows: usize, height: usize) -> usize {
    if rows <= height {
        return 0;
    }
    let mut top = top.min(rows + 1 - height);
    if row < top {
        top = row;
    }
    let room = if top > 0 { height - 1 } else { height };
    if row >= top + room {
        top = row + 2 - height;
    }
    top
}

fn wrap(text: &str, width: usize) -> Vec<(usize, usize)> {
    let mut rows = Vec::new();
    let mut line_start = 0;
    for line in text.split('\n') {
        let end = line_start + line.len();
        let mut start = line_start;
        let mut space = None;
        for (offset, c) in line.char_indices() {
            let at = line_start + offset;
            let wanted = text[start..at].width() + c.to_string().width();
            if wanted > width && at > start {
                match space.filter(|&space| space > start) {
                    Some(space) => {
                        rows.push((start, space));
                        start = space + 1;
                    }
                    None => {
                        rows.push((start, at));
                        start = at;
                    }
                }
                space = None;
            }
            if c == ' ' {
                space = Some(at);
            }
        }
        rows.push((start, end));
        line_start = end + 1;
    }
    rows
}
