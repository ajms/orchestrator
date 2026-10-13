use crossterm::event::KeyCode;
use orch_protocol::{AgentStateView, GuardChoice, GuardKindView, GuardPrompt, Request};

use crate::common::*;

fn guarded() -> Harness {
    let mut view = with_agent(session("webshop", "guarded"), AgentStateView::NeedsInput);
    view.guard_prompts = vec![GuardPrompt {
        id: 7,
        tool: "Bash".into(),
        kind: GuardKindView::BaseBranch,
        target: "git push origin main".into(),
    }];
    let mut tui = Harness::new();
    tui.sessions(vec![view, session("webshop", "calm")]);
    tui
}

fn answers(tui: &mut Harness) -> Vec<Request> {
    tui.daemon()
        .requests()
        .into_iter()
        .filter(|request| matches!(request, Request::AnswerGuard { .. }))
        .collect()
}

#[test]
fn a_guard_hit_on_the_selected_session_shows_a_prompt() {
    let mut tui = guarded();
    let screen = tui.screen();
    assert!(screen.contains("Guard"), "{screen}");
    assert!(screen.contains("git push origin main"), "{screen}");
    assert!(screen.contains("Base branch"), "{screen}");
    assert!(screen.contains("allow once"), "{screen}");
    assert!(screen.contains("allow for Session"), "{screen}");
    assert!(screen.contains("deny"), "{screen}");
}

#[test]
fn a_guard_on_a_long_target_still_shows_the_keys() {
    let mut view = with_agent(session("webshop", "guarded"), AgentStateView::NeedsInput);
    view.guard_prompts = vec![GuardPrompt {
        id: 7,
        tool: "Bash".into(),
        kind: GuardKindView::WriteOutsideWorktree,
        target: "cat notes.txt > /home/someone/a/very/deep/directory/structure/outside/of/the/worktree/notes.txt".into(),
    }];
    let mut tui = Harness::new();
    tui.sessions(vec![view]);
    let screen = tui.screen();
    assert!(screen.contains("3 deny · Esc later"), "{screen}");
}

#[test]
fn each_answer_is_sent_to_the_daemon() {
    for (key, choice) in [
        ('1', GuardChoice::AllowOnce),
        ('2', GuardChoice::AllowForSession),
        ('3', GuardChoice::Deny),
    ] {
        let mut tui = guarded();
        tui.keys(&key.to_string());
        assert_eq!(
            answers(&mut tui),
            vec![Request::AnswerGuard {
                session: id("guarded"),
                guard: 7,
                choice
            }]
        );
        assert!(!tui.screen().contains("allow for Session"));
    }
}

#[test]
fn the_prompt_takes_keys_even_in_insert_mode() {
    let mut tui = guarded();
    tui.press(KeyCode::Esc);
    tui.keys("i");
    let mut view = with_agent(session("webshop", "guarded"), AgentStateView::NeedsInput);
    view.guard_prompts = vec![GuardPrompt {
        id: 8,
        tool: "Write".into(),
        kind: GuardKindView::WriteOutsideWorktree,
        target: "/etc/hosts".into(),
    }];
    tui.changed(view);
    tui.keys("3");

    assert!(tui.daemon().input.is_empty());
    assert_eq!(
        answers(&mut tui),
        vec![Request::AnswerGuard {
            session: id("guarded"),
            guard: 8,
            choice: GuardChoice::Deny
        }]
    );
}

#[test]
fn esc_hides_the_prompt() {
    let mut tui = guarded();
    tui.press(KeyCode::Esc);
    assert!(!tui.screen().contains("allow for Session"));
}

#[test]
fn another_sessions_guard_hit_does_not_pop_up() {
    let mut tui = guarded();
    tui.keys("2");
    tui.keys("j");
    assert!(!tui.screen().contains("allow for Session"));
}

#[test]
fn a_hidden_prompt_is_announced_and_comes_back_with_the_guard_command() {
    let mut tui = guarded();
    tui.press(KeyCode::Esc);
    let status = statusline(&mut tui);
    assert!(status.contains("Guard prompt waiting"), "{status}");

    tui.command("guard");
    assert!(tui.screen().contains("allow for Session"));
}

#[test]
fn reselecting_the_session_brings_a_hidden_prompt_back() {
    let mut tui = guarded();
    tui.press(KeyCode::Esc);
    tui.keys("jk");
    assert!(tui.screen().contains("allow for Session"));
}
