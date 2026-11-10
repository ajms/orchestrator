use orch_protocol::{
    PhaseView, SessionView, SubagentMergeView, SubagentStateView, SubagentView,
    SubagentWorktreeView,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph, Wrap};
use tui_term::widget::PseudoTerminal;

use super::style;
use crate::app::{App, Focus, Mode, Selection};
use crate::hyperlinks::Hyperlink;
use crate::layout::Areas;
use crate::pane::PaneMirror;
use crate::preparing::Preparing;
use crate::selection::{Point, columns_on};

const SELECTION: Color = Color::Rgb(70, 70, 110);
use crate::sessions::repo_name;
use crate::shells::{Shell, ShellKind};

pub(super) fn draw(app: &App, frame: &mut Frame, areas: &Areas) {
    let area = areas.pane;
    let focused = app.focus == Focus::Pane;
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(super::border(focused, app.inserting()));
    if let Some(preparing) = app.cursor_preparing() {
        let block = block.title(preparing_title(preparing));
        render_tail(frame, area, block, preparing_lines(preparing));
        return;
    }
    if let Some((view, subagent)) = app.cursor_subagent() {
        frame.render_widget(block.title(subagent_title(view, subagent)), area);
        transcript(app, subagent, frame, areas);
        return;
    }
    let Some(view) = app.selected_view() else {
        let text = match &app.sidebar.heading {
            Some(repo) => format!(" {} is folded — Enter or za unfolds it", repo_name(repo)),
            None => " No Sessions yet — :new starts one".into(),
        };
        frame.render_widget(Paragraph::new(text).block(block), area);
        return;
    };
    block = match app.shown_shell() {
        Some(shell) => block.title(shell_title(view, shell)),
        None => block.title(title(view)),
    };
    match app.shown_pane() {
        Some(pane) => {
            let inner = block.inner(area);
            frame.render_widget(PseudoTerminal::new(pane.screen()).block(block), area);
            if let Some(selection) = shown_selection(app) {
                highlight(frame, inner, &selection, pane);
            }
            if let Some(link) = app.links.hovered() {
                underline(frame, inner, link, pane);
            }
        }
        None => {
            let closed = app
                .pane
                .as_ref()
                .filter(|pane| pane.session == view.id)
                .and_then(|pane| pane.closed.clone());
            render_tail(frame, area, block, placeholder(view, closed));
        }
    }
}

fn render_tail(frame: &mut Frame, area: Rect, block: Block, mut lines: Vec<Line<'static>>) {
    let inner = block.inner(area);
    let hidden = lines.len().saturating_sub(inner.height.into());
    let text = Paragraph::new(lines.split_off(hidden)).wrap(Wrap { trim: false });
    let overflow = text
        .line_count(inner.width)
        .saturating_sub(inner.height.into());
    let top = u16::try_from(overflow).unwrap_or(u16::MAX);
    frame.render_widget(text.scroll((top, 0)).block(block), area);
}

fn shown_selection(app: &App) -> Option<Selection> {
    match &app.mode {
        Mode::Visual(selection) => Some(*selection),
        _ => app.pane_selection.span().map(|(anchor, cursor)| Selection {
            linewise: false,
            anchor,
            cursor,
        }),
    }
}

fn highlight(frame: &mut Frame, inner: Rect, selection: &Selection, pane: &PaneMirror) {
    let span = selection.ordered();
    paint_selection(frame, inner, pane.line_of(0), span, selection.linewise);
}

pub(super) fn paint_selection(
    frame: &mut Frame,
    inner: Rect,
    top: i64,
    span: (Point, Point),
    linewise: bool,
) {
    let buffer = frame.buffer_mut();
    for_each_cell(inner, top, span, linewise, |at| {
        buffer[at].set_bg(SELECTION);
    });
}

fn underline(frame: &mut Frame, inner: Rect, link: &Hyperlink, pane: &PaneMirror) {
    let buffer = frame.buffer_mut();
    for_each_cell(
        inner,
        pane.line_of(0),
        (link.start, link.end),
        false,
        |at| {
            buffer[at].modifier.insert(Modifier::UNDERLINED);
        },
    );
}

fn for_each_cell(
    inner: Rect,
    top: i64,
    (start, end): (Point, Point),
    linewise: bool,
    mut paint: impl FnMut((u16, u16)),
) {
    let last_col = inner.width.saturating_sub(1);
    let first = (start.0 - top).max(0);
    let last = (end.0 - top).min(i64::from(inner.height) - 1);
    for row in first..=last {
        let line = top + row;
        let (from, to) = match linewise {
            true => (0, last_col),
            false => columns_on(line, (start, end), last_col),
        };
        for col in from..=to {
            paint((inner.x + col, inner.y + row as u16));
        }
    }
}

fn title(view: &SessionView) -> Line<'static> {
    let (label, colour) = style::status(view);
    let mode = match &view.mode {
        Some(mode) => format!("[{} · {mode}]", view.preset),
        None => format!("[{}]", view.preset),
    };
    let title = match &view.title {
        Some(title) => format!("{title} ({})", view.slug),
        None => view.slug.clone(),
    };
    let mut spans = vec![
        Span::raw(" "),
        Span::raw(format!("{} / {title} ", repo_name(&view.repo))).bold(),
        Span::styled(label, Style::new().fg(colour)),
        Span::raw(format!(" {mode} ")),
    ];
    if view.unsandboxed {
        spans.push(Span::styled("unsandboxed ", Style::new().fg(Color::Yellow)));
    }
    Line::from(spans)
}

fn shell_title(view: &SessionView, shell: &Shell) -> Line<'static> {
    let label = match &shell.kind {
        ShellKind::Command(command) => format!("shell: {command}"),
        ShellKind::Interactive => "shell".into(),
    };
    let mut spans = vec![
        Span::raw(" "),
        Span::raw(format!(
            "{} / {} ",
            repo_name(&view.repo),
            view.title_or_slug()
        ))
        .bold(),
        Span::styled(label, Style::new().fg(Color::Yellow)),
        Span::raw(" "),
    ];
    if let Some(exit) = &shell.exit {
        spans.push(Span::raw(format!("· {exit} · Enter closes ")).dark_gray());
    }
    Line::from(spans)
}

fn subagent_title(view: &SessionView, subagent: &SubagentView) -> Line<'static> {
    Line::from(vec![
        Span::raw(" "),
        Span::raw(format!(
            "{} / {} ↳ {} ",
            repo_name(&view.repo),
            view.title_or_slug(),
            subagent.agent_type
        ))
        .bold(),
    ])
}

fn transcript(app: &App, subagent: &SubagentView, frame: &mut Frame, areas: &Areas) {
    let header = Paragraph::new(subagent_header(subagent));
    frame.render_widget(header, areas.transcript_header());
    let body = areas.transcript_body();
    let outcome = subagent_outcome(subagent, body.height);
    let footer_height = u16::try_from(outcome.len()).unwrap_or(0);
    let (body, footer) = match footer_height {
        0 => (body, None),
        _ => {
            let [body, footer] =
                Layout::vertical([Constraint::Min(0), Constraint::Length(footer_height)])
                    .areas(body);
            (body, Some(footer))
        }
    };
    if let Some(footer) = footer {
        frame.render_widget(Paragraph::new(outcome), footer);
    }
    let Some(transcript) = &app.transcript else {
        return;
    };
    let height = usize::from(body.height);
    let rows = transcript.rows(body.width);
    if rows.is_empty() {
        let empty = Line::from("No Subagent transcript yet").dark_gray();
        frame.render_widget(Paragraph::new(empty), body);
        return;
    }
    let top = transcript.top(rows.len(), height);
    let shown: Vec<Line> = rows.iter().skip(top).take(height).cloned().collect();
    frame.render_widget(Paragraph::new(shown), body);
}

fn subagent_header(subagent: &SubagentView) -> Line<'static> {
    let name = match subagent.description.is_empty() {
        true => subagent.agent_type.clone(),
        false => format!("{}: {}", subagent.agent_type, subagent.description),
    };
    let state = match subagent.state {
        SubagentStateView::Working => Span::raw("running").green(),
        SubagentStateView::Done => Span::raw("done").dark_gray(),
        SubagentStateView::Errored => Span::raw("errored").red(),
    };
    let mut spans = vec![
        Span::raw(name).bold(),
        Span::raw(" · "),
        state,
        Span::raw(format!(" · {} tools", subagent.tool_count)),
    ];
    if let Some(duration) = subagent.duration_ms {
        spans.push(Span::raw(format!(" · {}", duration_text(duration))));
    }
    if let Some(model) = &subagent.model {
        spans.push(Span::raw(format!(" · {model}")).dark_gray());
    }
    if let Some(cost) = subagent.usage.as_ref().and_then(|usage| usage.cost_usd) {
        spans.push(Span::raw(format!(" · ${cost:.2}")).dark_gray());
    }
    Line::from(spans)
}

/// The lines under a finished Subagent's transcript: why it failed, what
/// became of its worktree, and its final text; at most a third of `height`.
fn subagent_outcome(subagent: &SubagentView, height: u16) -> Vec<Line<'static>> {
    if !subagent.finished() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    if let Some(failure) = &subagent.failure {
        lines.push(Line::from(vec![
            Span::raw("✗ ").red(),
            Span::raw(failure.clone()).red(),
        ]));
    }
    if let Some(worktree) = &subagent.worktree {
        lines.push(worktree_line(worktree));
    }
    if let Some(summary) = &subagent.summary {
        let label = "result";
        for (at, text) in summary
            .lines()
            .filter(|line| !line.trim().is_empty())
            .enumerate()
        {
            let prefix = match at {
                0 => format!("{label}: "),
                _ => " ".repeat(label.len() + 2),
            };
            lines.push(Line::from(vec![
                Span::raw(prefix).dark_gray(),
                Span::raw(text.to_string()),
            ]));
        }
    }
    let room = usize::from(height / 3);
    if lines.is_empty() || room < 2 {
        return Vec::new();
    }
    lines.truncate(room - 1);
    lines.insert(0, Line::from("─".repeat(usize::from(u8::MAX))).dark_gray());
    lines
}

fn worktree_line(worktree: &SubagentWorktreeView) -> Line<'static> {
    let mut spans = vec![Span::raw("worktree ").dark_gray()];
    if let Some(branch) = &worktree.branch {
        spans.push(Span::raw(branch.clone()));
    }
    let merge = worktree.merge.map(|merge| match merge {
        SubagentMergeView::Merged => Span::raw(" · merged").green(),
        SubagentMergeView::NothingToMerge => Span::raw(" · nothing to merge").dark_gray(),
        SubagentMergeView::NotMerged => Span::raw(" · not merged").red(),
    });
    spans.extend(merge);
    if let Some(kept) = &worktree.preserved_branch {
        spans.push(Span::raw(format!(" · kept on {kept}")).yellow());
    }
    if worktree.merge != Some(SubagentMergeView::Merged)
        && let Some(message) = &worktree.message
    {
        spans.push(Span::raw(format!(" · {message}")).dark_gray());
    }
    Line::from(spans)
}

fn duration_text(ms: u64) -> String {
    let seconds = ms / 1000;
    match seconds {
        0 => format!("{ms}ms"),
        1..60 => format!("{seconds}s"),
        60..3600 => format!("{}m {}s", seconds / 60, seconds % 60),
        _ => format!("{}h {}m", seconds / 3600, seconds % 3600 / 60),
    }
}

fn preparing_title(preparing: &Preparing) -> Line<'static> {
    Line::from(vec![
        Span::raw(" "),
        Span::raw(format!(
            "{} / {} ",
            repo_name(&preparing.create.repo),
            preparing.slug
        ))
        .bold(),
        match preparing.failure() {
            Some(_) => Span::raw("Preparing failed ").red(),
            None => Span::raw("Preparing ").cyan(),
        },
    ])
}

fn preparing_lines(preparing: &Preparing) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(preparing.create.repo.display().to_string()).dark_gray(),
        Line::default(),
    ];
    lines.extend(
        preparing
            .create
            .prompt
            .lines()
            .map(|line| Line::from(line.to_string())),
    );
    lines.push(Line::default());
    match preparing.failure() {
        Some(reason) => {
            lines.push(Line::from(reason.to_string()).red());
            lines.push(Line::default());
            lines.push(Line::from("Enter reopens the form · x dismisses").yellow());
        }
        None => lines.push(Line::from("Preparing the Worktree…").yellow()),
    }
    lines
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
        PhaseView::SetupFailed => "Setup failed — :retry · :start (anyway) · :d(iscard)",
        PhaseView::Suspended => "Suspended — i / :resume continues the Conversation",
        PhaseView::Landed => "Landed",
        PhaseView::Discarded => "Discarded",
        PhaseView::Active | PhaseView::PrOpen => "i / :resume restarts the Agent",
    };
    lines.push(Line::from(hint).yellow());
    lines
}
