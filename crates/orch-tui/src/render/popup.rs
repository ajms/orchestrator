use orch_protocol::{
    GuardKindView, GuardPrompt, LandingMode, RepoUsage, SessionView, UsageReport, UsageTotalsView,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};

use crate::app::{App, Popup};
use crate::discard::DiscardConfirm;
use crate::land::LandForm;
use crate::reconcile::RetargetPicker;
use crate::sessions::repo_name;

pub(super) fn draw(app: &App, frame: &mut Frame) {
    match &app.popup {
        Some(Popup::New(form)) => super::new_form::draw(frame, form, app.config.home.as_deref()),
        Some(Popup::Trust(prompt)) => trust(
            frame,
            &prompt.repo,
            &prompt.items,
            prompt.skipping_teardown().is_some(),
        ),
        Some(Popup::Land(form)) => {
            if let Some(view) = app.sessions.get(&form.session) {
                land(frame, view, form);
            }
        }
        Some(Popup::Discard(confirm)) => discard(frame, confirm),
        Some(Popup::Usage(report)) => usage(frame, report),
        Some(Popup::Retarget(picker)) => retarget(frame, picker),
        None => {}
    }
    if let Some((_, prompt)) = app.guard_prompt() {
        guard(frame, prompt);
    }
}

fn land(frame: &mut Frame, view: &SessionView, form: &LandForm) {
    let squash = form.mode == LandingMode::Squash;
    let choice = |text: String, chosen: bool| match chosen {
        true => Span::styled(text, Style::new().fg(Color::Black).bg(Color::Green)),
        false => Span::raw(text),
    };
    let mut lines = vec![
        Line::from(format!(
            " Land {} / {} into {}",
            repo_name(&view.repo),
            view.slug,
            view.base
        ))
        .bold(),
        Line::default(),
        Line::from(vec![
            Span::raw(" Tab: "),
            choice(format!("squash onto {}", view.base), squash),
            Span::raw("  "),
            choice("push + PR".into(), !squash),
        ]),
        Line::default(),
        Line::from(match (form.drafting, squash) {
            (true, _) => " The Agent is drafting…",
            (false, true) => " Commit message:",
            (false, false) => " PR title, then body:",
        })
        .dark_gray(),
    ];
    let text = format!("{}▏", form.message);
    lines.extend(text.split('\n').map(|line| Line::from(format!(" {line}"))));
    lines.push(Line::default());
    lines.push(Line::from(" Enter land · Ctrl+g $EDITOR · Esc cancel").dark_gray());
    show(frame, " :land ", Color::Green, lines, 76);
}

fn discard(frame: &mut Frame, preview: &DiscardConfirm) {
    let mut lines = vec![
        Line::from(format!(" {}", preview.question)).bold(),
        Line::default(),
    ];
    if preview.uncommitted.is_empty() && preview.unlanded.is_empty() {
        lines.push(Line::from(" nothing uncommitted and no unlanded commits").dark_gray());
    } else {
        lines.push(Line::from(" You will lose:").red());
        lines.extend(
            preview
                .uncommitted
                .iter()
                .map(|file| Line::from(format!("   {file} (uncommitted)"))),
        );
        lines.extend(
            preview
                .unlanded
                .iter()
                .map(|commit| Line::from(format!("   {commit}"))),
        );
    }
    lines.push(Line::default());
    lines.push(Line::from(" y discard · any other key cancels").dark_gray());
    show(frame, " :discard ", Color::Red, lines, 76);
}

fn usage(frame: &mut Frame, report: &UsageReport) {
    let mut lines = vec![
        Line::from(" Estimated from the Agents' own reports; actual billing may differ.")
            .dark_gray(),
        Line::default(),
    ];
    lines.push(Line::from(" Per Repo").bold());
    lines.extend(report.per_repo.iter().map(repo_usage));
    lines.push(Line::default());
    lines.push(Line::from(" Today").bold());
    lines.extend(report.today.iter().map(repo_usage));
    lines.push(Line::default());
    lines.push(Line::from(" Total").bold());
    lines.extend(
        report
            .per_agent
            .iter()
            .map(|total| usage_line("", &total.agent, &total.totals).bold()),
    );
    lines.push(Line::default());
    lines.push(Line::from(" Esc close").dark_gray());
    show(frame, " :usage (estimates) ", Color::Cyan, lines, 92);
}

fn repo_usage(usage: &RepoUsage) -> Line<'static> {
    usage_line(&repo_name(&usage.repo), &usage.agent, &usage.totals)
}

fn usage_line(name: &str, agent: &str, totals: &UsageTotalsView) -> Line<'static> {
    let cost = totals
        .cost_usd
        .map_or_else(|| "cost unknown".into(), |cost| format!("${cost:.2}"));
    Line::from(format!(
        "   {name:<20}  {agent:<12}  {:>8} in  {:>8} out  {cost}",
        tokens(totals.input_tokens),
        tokens(totals.output_tokens),
    ))
}

fn tokens(count: u64) -> String {
    match count {
        count if count >= 1_000_000 => format!("{:.1}M", count as f64 / 1_000_000.0),
        count if count >= 1_000 => format!("{:.1}k", count as f64 / 1_000.0),
        count => count.to_string(),
    }
}

fn retarget(frame: &mut Frame, picker: &RetargetPicker) {
    let base = picker
        .candidates
        .get(picker.choice)
        .cloned()
        .unwrap_or_default();
    let lines = vec![
        Line::from(format!(" Retarget {} onto:", picker.slug)).bold(),
        Line::default(),
        Line::from(format!("   ◂ {base} ▸")),
        Line::default(),
        Line::from(" ←/→ choose a Branch · Enter retarget · Esc cancel").dark_gray(),
    ];
    show(frame, " Retarget ", Color::Yellow, lines, 70);
}

pub(super) fn mismatch(frame: &mut Frame, message: &str) {
    let lines = vec![
        Line::from(format!(" {message}")),
        Line::default(),
        Line::from(" Live Sessions survive a Daemon restart.").dark_gray(),
        Line::from(" r restart the Daemon · q quit").bold(),
    ];
    show(frame, " Daemon version mismatch ", Color::Yellow, lines, 90);
}

fn trust(frame: &mut Frame, repo: &std::path::Path, items: &[String], skippable: bool) {
    let mut lines = vec![
        Line::from(format!(
            " {} brings scripts or Presets that need your Trust:",
            repo.display()
        )),
        Line::default(),
    ];
    lines.extend(
        items
            .iter()
            .map(|item| Line::from(format!("   {item}")).yellow()),
    );
    lines.push(Line::default());
    let keys = match skippable {
        true => " y trust and continue · s skip the Teardown · n cancel",
        false => " y trust and continue · n cancel",
    };
    lines.push(Line::from(keys).dark_gray());
    show(frame, " Trust ", Color::Yellow, lines, 84);
}

fn guard(frame: &mut Frame, prompt: &GuardPrompt) {
    let lines = vec![
        guard_heading(prompt),
        Line::default(),
        Line::from(format!("   {}", prompt.target)).yellow(),
        Line::default(),
        Line::from(" 1 allow once · 2 allow for Session · 3 deny · Esc later").dark_gray(),
    ];
    show(frame, " Guard ", Color::Yellow, lines, 70);
}

fn guard_heading(prompt: &GuardPrompt) -> Line<'static> {
    let on = |target: &str| {
        Line::from(vec![
            Span::raw(" The Agent wants to use "),
            Span::raw(prompt.tool.clone()).bold(),
            Span::raw(format!(" on {target}:")),
        ])
    };
    match prompt.kind {
        GuardKindView::BaseBranch => on("the Base branch"),
        GuardKindView::OtherRef => on("another ref"),
        GuardKindView::WorktreeManagement => on("worktrees"),
        GuardKindView::WriteOutsideWorktree => on("files outside the Worktree"),
        GuardKindView::ExternalTool => {
            Line::from(" The Agent wants to use a tool that reaches beyond the Session:")
        }
    }
}

fn show(frame: &mut Frame, title: &str, colour: Color, lines: Vec<Line<'static>>, width: u16) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(colour))
        .title(title.to_string());
    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(block);
    let width = width.min(frame.area().width);
    let height = paragraph.line_count(width.saturating_sub(2)) as u16;
    let area = centered(frame.area(), width, height);
    frame.render_widget(Clear, area);
    frame.render_widget(paragraph, area);
}

pub(super) fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Length(width.min(area.width))])
        .flex(Flex::Center)
        .areas(area);
    let [area] = Layout::vertical([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .areas(area);
    area
}
