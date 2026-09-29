use orch_protocol::{AgentStateView, PhaseView, SessionView};
use ratatui::style::Color;

pub fn status(view: &SessionView) -> (&'static str, Color) {
    match (view.phase, view.agent) {
        (PhaseView::Active | PhaseView::PrOpen, Some(agent)) => agent_state(agent),
        (phase, _) => phase_label(phase),
    }
}

pub fn agent_state(state: AgentStateView) -> (&'static str, Color) {
    match state {
        AgentStateView::Starting => ("Starting", Color::Cyan),
        AgentStateView::Working => ("Working", Color::Green),
        AgentStateView::NeedsInput => ("Needs input", Color::Yellow),
        AgentStateView::Idle => ("Idle", Color::Blue),
        AgentStateView::Errored => ("Errored", Color::Red),
        AgentStateView::Exited => ("Exited", Color::DarkGray),
        AgentStateView::Unknown => ("Unknown", Color::Gray),
    }
}

fn phase_label(phase: PhaseView) -> (&'static str, Color) {
    let colour = match phase {
        PhaseView::SettingUp => Color::Cyan,
        PhaseView::SetupFailed => Color::Red,
        PhaseView::Active => Color::Gray,
        PhaseView::PrOpen => Color::Magenta,
        PhaseView::Suspended | PhaseView::Landed | PhaseView::Discarded => Color::DarkGray,
    };
    (crate::sessions::phase_label(phase), colour)
}

pub fn context(percent: f64) -> Color {
    match percent {
        p if p >= 90.0 => Color::Red,
        p if p >= 80.0 => Color::Yellow,
        _ => Color::DarkGray,
    }
}
