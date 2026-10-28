use crossterm::event::{Event as TermEvent, KeyCode};
use orch_protocol::{BranchFound, BranchLookup, LookupDepth, OpenPr};
use orch_tui::Event;

use crate::common::*;
use crate::new_session::{config, field, form};

fn on_branch_field() -> Harness {
    let mut tui = form(config());
    tui.keys("Fix");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui
}

fn replace_branch(tui: &mut Harness, name: &str) {
    tui.send(Event::Terminal(TermEvent::Paste(name.into())));
}

fn status(tui: &mut Harness) -> String {
    let lines = tui.lines();
    let at = lines
        .iter()
        .position(|line| line.contains("│ Branch "))
        .unwrap_or_else(|| panic!("no Branch field in\n{}", lines.join("\n")));
    lines[at + 2..]
        .iter()
        .map(|line| line.split('│').nth(2).unwrap_or_default().trim())
        .take_while(|field| !field.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn existing(branch: &str, found: BranchFound, pr: Option<OpenPr>) -> BranchLookup {
    BranchLookup::Existing {
        branch: branch.into(),
        found,
        pr,
    }
}

fn open_pr(base: &str) -> Option<OpenPr> {
    Some(OpenPr {
        number: 123,
        title: "Add retry".into(),
        base: base.into(),
    })
}

fn answered(typed: &str, lookup: BranchLookup) -> Harness {
    let mut tui = on_branch_field();
    tui.daemon().origin_lookups.push((typed.into(), lookup));
    tui.keys(typed);
    tui.later(500);
    tui.tick();
    tui
}

#[test]
fn the_status_line_under_branch_says_what_the_typed_name_is() {
    let cases = [
        (
            "fresh",
            BranchLookup::New {
                base: "main".into(),
            },
            "+ new Branch from main",
        ),
        (
            "feature-x",
            existing("feature-x", BranchFound::Local, None),
            "↺ Existing Branch · local · no PR",
        ),
        (
            "remote-y",
            existing("remote-y", BranchFound::Origin, None),
            "↺ Existing Branch · origin · no PR",
        ),
        (
            "#123",
            existing("feature-x", BranchFound::Local, open_pr("main")),
            "↺ feature-x · PR #123 open \"Add retry\"",
        ),
        (
            "main",
            BranchLookup::Refused {
                reason: "main is the Session's Base branch".into(),
            },
            "✗ main is the Session's Base branch",
        ),
    ];
    for (typed, lookup, shown) in cases {
        let mut tui = answered(typed, lookup);
        let line = status(&mut tui);
        assert!(line.contains(shown), "{typed}: {line}");
    }
}

#[test]
fn erasing_a_typed_branch_leaves_it_empty_showing_the_derived_name_and_typeable() {
    let mut tui = on_branch_field();
    tui.keys("ab");
    tui.press(KeyCode::Backspace);
    tui.press(KeyCode::Backspace);

    assert_eq!(tui.colour_of("orch/fix"), ratatui::style::Color::DarkGray);
    assert!(status(&mut tui).contains("+ new Branch from"));
    tui.keys("c");
    tui.press(KeyCode::Tab);
    tui.ctrl('s');
    let create = create_requests(&mut tui).pop().expect("no Session created");
    assert_eq!(create.branch.as_deref(), Some("c"));
}

#[test]
fn a_derived_branch_is_new_from_the_base_without_a_lookup() {
    let mut tui = form(config());
    tui.keys("Fix");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.keys("release");
    tui.later(500);
    tui.tick();

    assert!(status(&mut tui).contains("+ new Branch from release"));
    assert!(tui.daemon().lookups().is_empty());
}

#[test]
fn origin_is_asked_after_half_a_second_without_typing() {
    let mut tui = on_branch_field();
    tui.keys("feat");
    tui.later(300);
    tui.tick();
    tui.keys("ure");
    tui.later(300);
    tui.tick();

    assert!(status(&mut tui).contains("⋯ looking on origin…"));
    assert!(
        !tui.daemon()
            .lookups()
            .contains(&("feature".into(), LookupDepth::Origin))
    );
    tui.later(200);
    tui.tick();
    assert!(status(&mut tui).contains("+ new Branch from main"));
    assert_eq!(
        tui.daemon()
            .lookups()
            .into_iter()
            .filter(|(_, depth)| *depth == LookupDepth::Origin)
            .collect::<Vec<_>>(),
        vec![("feature".to_string(), LookupDepth::Origin)]
    );
}

#[test]
fn origin_is_asked_at_once_when_focus_leaves_the_branch_field() {
    let mut tui = on_branch_field();
    replace_branch(&mut tui, "feature");
    tui.press(KeyCode::Tab);

    assert!(
        tui.daemon()
            .lookups()
            .contains(&("feature".into(), LookupDepth::Origin))
    );
    assert!(status(&mut tui).contains("+ new Branch from main"));
}

#[test]
fn a_local_refusal_shows_on_the_keystroke() {
    let mut tui = on_branch_field();
    let refusal = BranchLookup::Refused {
        reason: "checked out in the main checkout".into(),
    };
    tui.daemon().local_lookups.push(("work".into(), refusal));
    tui.keys("work");

    assert!(status(&mut tui).contains("✗ checked out in the main checkout"));
    assert_eq!(
        tui.daemon().lookups().last(),
        Some(&("work".into(), LookupDepth::Local))
    );
}

#[test]
fn a_reply_for_an_older_branch_value_is_ignored() {
    let mut tui = on_branch_field();
    tui.daemon().hold_lookups = true;
    let refusal = BranchLookup::Refused {
        reason: "feature is the Branch of Session other".into(),
    };
    tui.daemon()
        .origin_lookups
        .push(("feature".into(), refusal));
    tui.keys("feature");
    tui.later(500);
    tui.tick();
    tui.keys("-x");
    tui.release_replies();

    assert!(status(&mut tui).contains("⋯ looking on origin…"));
}

fn on_open_pr() -> Harness {
    let pr = existing("feature-x", BranchFound::Local, open_pr("release"));
    let mut tui = answered("#123", pr);
    tui.press(KeyCode::Tab);
    tui
}

#[test]
fn with_an_open_pr_the_base_shows_the_prs_base_and_creating_sends_it() {
    let mut tui = on_open_pr();

    assert!(field(&mut tui, "Base").contains("release (from PR #123)"));
    tui.ctrl('s');
    let create = create_requests(&mut tui).pop().expect("no Session created");
    assert_eq!(create.branch.as_deref(), Some("#123"));
    assert_eq!(create.base.as_deref(), Some("release"));
}

#[test]
fn the_base_from_an_open_pr_ignores_typing_until_the_branch_changes() {
    let mut tui = on_open_pr();
    tui.keys("dev");
    tui.press(KeyCode::BackTab);
    replace_branch(&mut tui, "other");
    tui.press(KeyCode::Tab);

    let base = field(&mut tui, "Base");
    assert!(!base.contains("dev"), "{base}");
    tui.keys("main");
    assert!(field(&mut tui, "Base").contains("main"));
}

#[test]
fn ctrl_s_is_blocked_while_a_refusal_shows() {
    let refusal = BranchLookup::Refused {
        reason: "diverged from origin: pull or push".into(),
    };
    let mut tui = answered("feature-x", refusal);
    tui.ctrl('s');

    assert!(create_requests(&mut tui).is_empty());
    assert!(tui.screen().contains("Ctrl+s blocked"));
}

fn waiting_on(lookup: BranchLookup) -> Harness {
    let mut tui = on_branch_field();
    tui.daemon().hold_lookups = true;
    tui.daemon()
        .origin_lookups
        .push(("feature-x".into(), lookup));
    tui.keys("feature-x");
    tui.ctrl('s');
    tui
}

#[test]
fn ctrl_s_during_a_lookup_waits_for_it_then_creates_the_session() {
    let mut tui = waiting_on(existing("feature-x", BranchFound::Origin, None));

    assert!(create_requests(&mut tui).is_empty());
    assert!(tui.screen().contains("⋯ checking before creating"));
    assert!(
        tui.daemon()
            .lookups()
            .contains(&("feature-x".into(), LookupDepth::Origin))
    );
    tui.release_replies();
    let create = create_requests(&mut tui).pop().expect("no Session created");
    assert_eq!(create.branch.as_deref(), Some("feature-x"));
}

#[test]
fn a_refusal_found_while_waiting_shows_instead_of_creating() {
    let refusal = BranchLookup::Refused {
        reason: "origin can't be reached".into(),
    };
    let mut tui = waiting_on(refusal);
    tui.release_replies();

    assert!(create_requests(&mut tui).is_empty());
    assert!(status(&mut tui).contains("✗ origin can't be reached"));
}

#[test]
fn a_failed_lookup_does_not_block_creation_and_leaves_the_decision_to_the_daemon() {
    let mut tui = on_branch_field();
    tui.daemon().lookups_fail = true;
    tui.keys("feature-x");
    tui.press(KeyCode::Tab);

    let line = status(&mut tui);
    assert!(line.contains("the lookup broke"), "{line}");
    assert!(!line.contains('✗'), "{line}");
    tui.ctrl('s');
    let create = create_requests(&mut tui).pop().expect("no Session created");
    assert_eq!(create.branch.as_deref(), Some("feature-x"));
}

#[test]
fn esc_cancels_the_wait_and_keeps_the_form_open() {
    let mut tui = waiting_on(existing("feature-x", BranchFound::Local, None));
    tui.press(KeyCode::Esc);
    tui.release_replies();

    assert!(create_requests(&mut tui).is_empty());
    let screen = tui.screen();
    assert!(screen.contains(":new Session"), "{screen}");
    assert!(!screen.contains("checking before creating"), "{screen}");
}
