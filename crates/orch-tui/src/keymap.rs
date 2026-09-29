use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use orch_protocol::{AgentStateView, PhaseView};
use orch_term::keys::{encode_key, is_ctrl_backslash};

use crate::app::{App, Call, Focus, Mode, Prefix, Selection};
use crate::event::{Effect, ReviewPurpose};
use crate::guard::GuardId;
use crate::layout::Columns;
use crate::reconcile::ReconcileAction;
use crate::sessions::phase_label;

pub(crate) fn handle(app: &mut App, key: KeyEvent) {
    if app.mismatch.is_some() {
        return mismatch(app, key);
    }
    if let Some((session, prompt)) = app.guard_prompt() {
        let guard = GuardId {
            session: session.clone(),
            guard: prompt.id,
        };
        return crate::popup::guard_key(app, guard, key);
    }
    if app.popup.is_some() {
        return crate::popup::key(app, key);
    }
    match &app.mode {
        Mode::Insert => insert(app, key),
        Mode::CommandLine(_) => command_line(app, key),
        Mode::Visual(selection) => visual(app, *selection, key),
        Mode::Normal if app.reconcile.is_some() => reconcile(app, key),
        Mode::Normal if app.review.is_some() => review(app, key),
        Mode::Normal => normal(app, key),
    }
}

fn mismatch(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('r') => {
            app.mismatch = Some("restarting the Daemon…".into());
            app.push(Call::Local(Effect::RestartDaemon));
        }
        KeyCode::Char('q') => app.push(Call::Local(Effect::Quit)),
        _ => {}
    }
}

fn reconcile(app: &mut App, key: KeyEvent) {
    app.message = None;
    let Some(view) = &mut app.reconcile else {
        return;
    };
    match view.key(key) {
        ReconcileAction::Stay => {}
        ReconcileAction::Close => app.reconcile = None,
        ReconcileAction::CommandLine => app.mode = Mode::CommandLine(String::new()),
        ReconcileAction::Apply(fix) => app.apply_fix(fix),
    }
}

fn review(app: &mut App, key: KeyEvent) {
    app.message = None;
    let body = Columns::of(app.areas().main()).diff_body();
    let Some(review) = &mut app.review else {
        return;
    };
    let action = review.key(key, body);
    app.review_action(action);
}

pub(crate) fn paste(app: &mut App, text: String) {
    if app.mode == Mode::Insert && app.pane.is_some() {
        app.push(Call::Paste(text));
    }
}

fn insert(app: &mut App, key: KeyEvent) {
    if app.prefix.take() == Some(Prefix::CtrlBackslash) {
        if ctrl(key) && key.code == KeyCode::Char('n') {
            app.mode = Mode::Normal;
            return;
        }
        send_key(
            app,
            KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL),
        );
    }
    if is_ctrl_backslash(key) {
        app.prefix = Some(Prefix::CtrlBackslash);
        return;
    }
    if ctrl(key) && key.code == KeyCode::Char('h') {
        app.mode = Mode::Normal;
        app.focus = Focus::Sidebar;
        return;
    }
    send_key(app, key);
}

fn send_key(app: &mut App, key: KeyEvent) {
    let Some(pane) = &app.pane else {
        return;
    };
    let bytes = encode_key(key, pane.input_modes());
    if !bytes.is_empty() {
        app.push(Call::Input(bytes));
    }
}

fn normal(app: &mut App, key: KeyEvent) {
    app.message = None;
    match app.prefix.take() {
        Some(Prefix::CtrlW) => return window(app, key),
        Some(Prefix::G) if key.code == KeyCode::Char('g') => return scroll_to(app, usize::MAX),
        _ => {}
    }
    let in_pane = app.focus == Focus::Pane;
    let half_page = (app.pane_size().rows / 2).max(1) as isize;
    match key.code {
        KeyCode::Char('w') if ctrl(key) => app.prefix = Some(Prefix::CtrlW),
        KeyCode::Char('u') if ctrl(key) => scroll_by(app, half_page),
        KeyCode::Char('d') if ctrl(key) => scroll_by(app, -half_page),
        KeyCode::Char('j') | KeyCode::Down if in_pane => scroll_by(app, -1),
        KeyCode::Char('k') | KeyCode::Up if in_pane => scroll_by(app, 1),
        KeyCode::Char('j') | KeyCode::Down => app.select_offset(1),
        KeyCode::Char('k') | KeyCode::Up => app.select_offset(-1),
        KeyCode::Enter | KeyCode::Char('l') => app.focus = Focus::Pane,
        KeyCode::Char('h') | KeyCode::Char('-') => app.focus = Focus::Sidebar,
        KeyCode::Char('g') => app.prefix = Some(Prefix::G),
        KeyCode::Char('G') => scroll_to(app, 0),
        KeyCode::Char('v') => start_visual(app, false),
        KeyCode::Char('V') => start_visual(app, true),
        KeyCode::Char('i') | KeyCode::Char('a') => enter_insert(app),
        KeyCode::Char('d') => app.load_review(ReviewPurpose::BuiltIn),
        KeyCode::Char('D') => app.load_review(ReviewPurpose::External),
        KeyCode::Char(':') => app.mode = Mode::CommandLine(String::new()),
        _ => {}
    }
}

fn command_line(app: &mut App, key: KeyEvent) {
    let Mode::CommandLine(line) = &mut app.mode else {
        return;
    };
    match key.code {
        KeyCode::Esc => app.mode = Mode::Normal,
        KeyCode::Enter => {
            let line = std::mem::take(line);
            app.mode = Mode::Normal;
            crate::commands::run(app, &line);
        }
        KeyCode::Backspace => {
            if line.pop().is_none() {
                app.mode = Mode::Normal;
            }
        }
        KeyCode::Char(c) => line.push(c),
        _ => {}
    }
}

fn scroll_by(app: &mut App, lines: isize) {
    if let Some(pane) = &mut app.pane {
        pane.scroll_by(lines);
    }
}

fn scroll_to(app: &mut App, offset: usize) {
    if let Some(pane) = &mut app.pane {
        pane.set_scroll(offset);
    }
}

fn window(app: &mut App, key: KeyEvent) {
    app.focus = match (key.code, app.focus) {
        (KeyCode::Char('h'), _) => Focus::Sidebar,
        (KeyCode::Char('l'), _) => Focus::Pane,
        (KeyCode::Char('w'), Focus::Sidebar) => Focus::Pane,
        (KeyCode::Char('w'), Focus::Pane) => Focus::Sidebar,
        (_, focus) => focus,
    };
}

fn start_visual(app: &mut App, linewise: bool) {
    let Some(pane) = &app.pane else {
        return;
    };
    let cursor = pane.cursor_line();
    app.pane_selection.clear();
    app.focus = Focus::Pane;
    app.mode = Mode::Visual(Selection {
        linewise,
        anchor: cursor,
        cursor,
    });
}

fn visual(app: &mut App, mut selection: Selection, key: KeyEvent) {
    let last_col = app.pane_size().cols.saturating_sub(1);
    let Some(pane) = &mut app.pane else {
        app.mode = Mode::Normal;
        return;
    };
    let (line, col) = &mut selection.cursor;
    match key.code {
        KeyCode::Char('h') | KeyCode::Left => *col = col.saturating_sub(1),
        KeyCode::Char('l') | KeyCode::Right => *col = (*col + 1).min(last_col),
        KeyCode::Char('k') | KeyCode::Up => *line = pane.reveal(*line - 1),
        KeyCode::Char('j') | KeyCode::Down => *line = pane.reveal(*line + 1),
        KeyCode::Char('0') => *col = 0,
        KeyCode::Char('$') => *col = last_col,
        KeyCode::Char('y') => {
            let text = selected_text(pane, selection, last_col);
            let lines = text.lines().count();
            app.copy(&text);
            app.message = Some(format!("yanked {lines} line(s)"));
            app.mode = Mode::Normal;
            return;
        }
        KeyCode::Esc | KeyCode::Char('v') | KeyCode::Char('V') => {
            app.mode = Mode::Normal;
            return;
        }
        _ => {}
    }
    app.mode = Mode::Visual(selection);
}

fn selected_text(
    pane: &mut crate::pane::PaneMirror,
    selection: Selection,
    last_col: u16,
) -> String {
    let (start, end) = selection.ordered();
    let text = match selection.linewise {
        true => pane.text_of_lines((start.0, 0), (end.0, last_col + 1)),
        false => pane.text_of_lines(start, (end.0, end.1 + 1)),
    };
    text.trim_end_matches('\n').to_string()
}

fn enter_insert(app: &mut App) {
    let Some(view) = app.selected_view() else {
        return;
    };
    let ended = matches!(
        view.agent,
        Some(AgentStateView::Exited | AgentStateView::Errored)
    );
    if view.phase == PhaseView::Suspended || (view.phase.is_live() && ended) {
        return app.resume_selected(true);
    }
    if !view.phase.is_live() {
        app.message = Some(format!("no live Agent while {}", phase_label(view.phase)));
        return;
    }
    scroll_to(app, 0);
    app.focus = Focus::Pane;
    app.mode = Mode::Insert;
}

fn ctrl(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
}
