use std::path::PathBuf;

use crossterm::event::{Event as TermEvent, KeyCode, MouseEventKind};
use orch_protocol::{FromDaemon, PhaseView, Request, Size};
use orch_tui::{Effect, Event, PaneId, ShellEvent};

use crate::common::*;

fn worktree(slug: &str) -> PathBuf {
    format!("/home/me/webshop/.orchestrator/worktrees/{slug}").into()
}

fn two_sessions() -> Harness {
    let mut tui = Harness::new();
    tui.sessions(vec![
        session("webshop", "first"),
        session("webshop", "second"),
    ]);
    tui.pane(
        "first",
        FromDaemon::Output {
            bytes: b"agent says hi".to_vec(),
        },
    );
    tui
}

fn opened_shell(tui: &mut Harness) -> (PaneId, Option<String>, PathBuf, Size) {
    tui.take_effects()
        .into_iter()
        .find_map(|effect| match effect {
            Effect::OpenShell {
                pane,
                command,
                cwd,
                size,
            } => Some((pane, command, cwd, size)),
            _ => None,
        })
        .expect("no shell opened")
}

fn opened_any(tui: &mut Harness) -> bool {
    tui.take_effects()
        .iter()
        .any(|effect| matches!(effect, Effect::OpenShell { .. }))
}

fn shell_input(effects: &[Effect], shell: PaneId) -> String {
    let bytes: Vec<u8> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::ShellInput { pane, bytes } if *pane == shell => Some(bytes.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn output(tui: &mut Harness, pane: PaneId, text: &str) {
    tui.send(Event::Shell {
        pane,
        event: ShellEvent::Output(text.as_bytes().to_vec()),
    });
}

fn exited(tui: &mut Harness, pane: PaneId, status: &str) {
    tui.send(Event::Shell {
        pane,
        event: ShellEvent::Exited(status.into()),
    });
}

fn open_sh(tui: &mut Harness) -> PaneId {
    tui.command("sh");
    let (pane, ..) = opened_shell(tui);
    output(tui, pane, "$ shell prompt");
    pane
}

fn to_normal(tui: &mut Harness) {
    tui.ctrl('\\');
    tui.ctrl('n');
}

#[test]
fn sh_opens_a_shell_in_the_worktree_sized_to_the_pane() {
    let mut tui = two_sessions();
    let pane_size = tui.daemon().opened.last().unwrap().1;
    tui.command("sh");
    let (_, command, cwd, size) = opened_shell(&mut tui);
    assert_eq!(command, None);
    assert_eq!(cwd, worktree("first"));
    assert_eq!(size, pane_size);
}

#[test]
fn the_shell_is_shown_in_the_pane_beside_the_sidebar() {
    let mut tui = two_sessions();
    open_sh(&mut tui);
    let screen = tui.screen();
    assert!(screen.contains("$ shell prompt"), "{screen}");
    assert!(!screen.contains("agent says hi"), "{screen}");
    tui.sidebar_line_with("second");
    assert!(tui.line_with("shell").contains("first"));
}

#[test]
fn keys_and_pastes_in_insert_mode_go_to_the_shell_not_the_agent() {
    let mut tui = two_sessions();
    let shell = open_sh(&mut tui);
    assert!(statusline(&mut tui).contains("INSERT"));
    tui.keys("ls\n");
    tui.send(Event::Terminal(TermEvent::Paste("pasted".into())));
    assert_eq!(shell_input(&tui.take_effects(), shell), "ls\rpasted");
    assert_eq!(tui.daemon().input_to("first"), "");
    assert!(tui.daemon().pastes.is_empty());
}

#[test]
fn sh_again_hides_the_shell_and_shows_it_without_starting_another() {
    let mut tui = two_sessions();
    open_sh(&mut tui);
    to_normal(&mut tui);
    tui.command("sh");
    assert!(tui.screen().contains("agent says hi"));
    assert!(statusline(&mut tui).contains("NORMAL"));

    tui.command("sh");
    assert!(tui.screen().contains("$ shell prompt"));
    assert!(statusline(&mut tui).contains("INSERT"));
    assert!(!opened_any(&mut tui));
}

#[test]
fn each_session_keeps_its_own_shell() {
    let mut tui = two_sessions();
    open_sh(&mut tui);
    tui.ctrl('h');
    tui.keys("j");
    assert!(!tui.screen().contains("$ shell prompt"));

    tui.keys("k");
    assert!(tui.screen().contains("$ shell prompt"));
}

#[test]
fn i_enters_the_shown_shell_even_without_a_live_agent() {
    let mut tui = Harness::new();
    tui.sessions(vec![in_phase(
        session("webshop", "first"),
        PhaseView::Suspended,
    )]);
    open_sh(&mut tui);
    to_normal(&mut tui);
    tui.keys("i");
    assert!(statusline(&mut tui).contains("INSERT"));
    let resumed = tui
        .daemon()
        .requests()
        .into_iter()
        .any(|request| matches!(request, Request::Resume { .. }));
    assert!(!resumed);
}

#[test]
fn exiting_the_shell_goes_back_to_the_agent() {
    let mut tui = two_sessions();
    let shell = open_sh(&mut tui);
    exited(&mut tui, shell, "exit status: 0");
    assert!(tui.screen().contains("agent says hi"));
    assert!(statusline(&mut tui).contains("NORMAL"));
    assert!(
        tui.take_effects()
            .contains(&Effect::CloseShell { pane: shell })
    );

    tui.command("sh");
    opened_shell(&mut tui);
}

#[test]
fn bang_runs_its_command_in_the_pane_and_keeps_the_output_until_enter() {
    let mut tui = two_sessions();
    tui.command("!cargo test");
    let (shell, command, cwd, _) = opened_shell(&mut tui);
    assert_eq!(command.as_deref(), Some("cargo test"));
    assert_eq!(cwd, worktree("first"));
    output(&mut tui, shell, "test result: ok");
    exited(&mut tui, shell, "exit status: 0");

    let screen = tui.screen();
    assert!(screen.contains("test result: ok"), "{screen}");
    assert!(screen.contains("exit status: 0"), "{screen}");
    assert!(statusline(&mut tui).contains("NORMAL"));

    tui.press(KeyCode::Enter);
    assert!(tui.screen().contains("agent says hi"));
}

#[test]
fn esc_also_dismisses_a_finished_command() {
    let mut tui = two_sessions();
    tui.command("!true");
    let (shell, ..) = opened_shell(&mut tui);
    exited(&mut tui, shell, "exit status: 0");
    tui.press(KeyCode::Esc);
    assert!(tui.screen().contains("agent says hi"));
}

#[test]
fn bang_is_refused_while_the_sessions_shell_runs() {
    let mut tui = two_sessions();
    open_sh(&mut tui);
    to_normal(&mut tui);
    tui.command("!ls");
    assert!(statusline(&mut tui).contains("a shell is running"));
    assert!(!opened_any(&mut tui));
}

#[test]
fn bang_without_a_command_shows_usage() {
    let mut tui = two_sessions();
    tui.take_effects();
    tui.command("!");
    assert!(statusline(&mut tui).contains("usage: :!<command>"));
    assert!(!opened_any(&mut tui));
}

#[test]
fn shell_commands_need_a_selected_session() {
    for line in ["!ls", "sh"] {
        let mut tui = Harness::new();
        tui.command(line);
        assert!(
            statusline(&mut tui).contains("no Session selected"),
            ":{line}"
        );
        assert!(!opened_any(&mut tui), ":{line}");
    }
}

#[test]
fn resizing_resizes_every_shell() {
    let mut tui = two_sessions();
    let shell = open_sh(&mut tui);
    tui.resize(100, 24);
    let resized = tui.take_effects().into_iter().any(|effect| {
        matches!(effect, Effect::ResizeShell { pane, size } if pane == shell && size.cols < 100)
    });
    assert!(resized);
}

#[test]
fn a_removed_session_closes_its_shell() {
    let mut tui = two_sessions();
    let shell = open_sh(&mut tui);
    tui.send(Event::Daemon(FromDaemon::SessionRemoved {
        session: id("first"),
    }));
    assert!(
        tui.take_effects()
            .contains(&Effect::CloseShell { pane: shell })
    );
}

#[test]
fn the_agents_pane_closing_keeps_you_typing_in_the_shell() {
    let mut tui = two_sessions();
    let shell = open_sh(&mut tui);
    tui.pane(
        "first",
        FromDaemon::PaneClosed {
            reason: "Agent exited".into(),
        },
    );
    assert!(statusline(&mut tui).contains("INSERT"));
    tui.keys("x");
    assert_eq!(shell_input(&tui.take_effects(), shell), "x");
}

#[test]
fn a_session_that_suspends_lands_or_is_discarded_closes_its_shell() {
    for phase in [
        PhaseView::Suspended,
        PhaseView::Landed,
        PhaseView::Discarded,
    ] {
        let mut tui = two_sessions();
        let shell = open_sh(&mut tui);
        tui.changed(in_phase(session("webshop", "first"), phase));
        assert!(
            tui.take_effects()
                .contains(&Effect::CloseShell { pane: shell }),
            "{phase:?}"
        );
        assert!(!tui.screen().contains("$ shell prompt"), "{phase:?}");
        assert!(statusline(&mut tui).contains("NORMAL"), "{phase:?}");
    }
}

#[test]
fn a_shell_opened_on_a_suspended_session_survives_its_updates() {
    let mut tui = Harness::new();
    let asleep = in_phase(session("webshop", "first"), PhaseView::Suspended);
    tui.sessions(vec![asleep.clone()]);
    let shell = open_sh(&mut tui);
    tui.changed(asleep);
    assert!(
        !tui.take_effects()
            .contains(&Effect::CloseShell { pane: shell })
    );
    assert!(tui.screen().contains("$ shell prompt"));
}

#[test]
fn the_wheel_scrolls_the_shell() {
    let mut tui = two_sessions();
    let shell = open_sh(&mut tui);
    let lines: String = (0..80).map(|n| format!("line {n:02}\r\n")).collect();
    output(&mut tui, shell, &lines);
    assert!(!tui.screen().contains("line 05"));
    for _ in 0..30 {
        tui.wheel(MouseEventKind::ScrollUp, PANE_LEFT + 5, PANE_TOP + 5);
    }
    assert!(tui.screen().contains("line 05"), "{}", tui.screen());
}
