use std::path::PathBuf;

use crossterm::event::KeyCode;
use orch_tui::{Effect, Event, FileDiff, ReviewData, ReviewPurpose, ReviewTarget, TuiConfig};
use ratatui::style::Color;

use crate::common::*;

fn reviewing() -> Harness {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first")]);
    tui.keys("d");
    tui.take_effects();
    tui.send(Event::Review {
        session: id("first"),
        purpose: ReviewPurpose::BuiltIn,
        result: Ok(data(vec![
            FileDiff {
                path: "src/login.rs".into(),
                lines: vec![
                    "@@ -1,3 +1,3 @@".into(),
                    " use std::time::Duration;".into(),
                    "-let timeout = 50;".into(),
                    "+let timeout = config.login_timeout();".into(),
                    "+log::debug!(\"timeout\");".into(),
                ],
                deleted: false,
            },
            FileDiff {
                path: "src/config.rs".into(),
                lines: vec!["@@ -0,0 +1 @@".into(), "+pub struct Config;".into()],
                deleted: false,
            },
        ])),
    });
    tui
}

fn data(files: Vec<FileDiff>) -> ReviewData {
    ReviewData {
        merge_base: "b45e".into(),
        tree: "7ree".into(),
        files,
    }
}

fn target() -> ReviewTarget {
    ReviewTarget {
        repo: PathBuf::from("/home/me/webshop"),
        worktree: PathBuf::from("/home/me/webshop/.orchestrator/worktrees/first"),
        slug: "first".into(),
        branch: "orch/first".into(),
        base: "main".into(),
    }
}

#[test]
fn d_asks_for_the_diff_of_the_worktree_against_its_base() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first")]);
    tui.keys("d");

    assert!(tui.take_effects().contains(&Effect::LoadReview {
        session: id("first"),
        target: target(),
        purpose: ReviewPurpose::BuiltIn,
    }));
}

#[test]
fn the_review_lists_files_with_counts_and_shows_a_coloured_diff() {
    let mut tui = reviewing();
    let screen = tui.screen();
    assert!(tui.line_with("src/login.rs").contains("+2 -1"), "{screen}");
    assert!(tui.line_with("src/config.rs").contains("+1 -0"), "{screen}");
    assert_eq!(tui.colour_of("+let timeout"), Color::Green);
    assert_eq!(tui.colour_of("-let timeout"), Color::Red);
    assert_eq!(tui.colour_of("@@ -1,3"), Color::Cyan);
    assert!(!screen.contains("pub struct Config"));
}

#[test]
fn j_and_k_move_between_files_and_q_closes_the_review() {
    let mut tui = reviewing();
    tui.keys("j");
    assert!(tui.screen().contains("+pub struct Config;"));
    tui.keys("k");
    assert!(tui.screen().contains("+let timeout"));
    tui.keys("q");
    assert!(!tui.screen().contains("src/login.rs"));
}

#[test]
fn review_command_opens_the_same_review() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first")]);
    tui.command("review");
    assert!(
        tui.take_effects()
            .iter()
            .any(|effect| matches!(effect, Effect::LoadReview { .. }))
    );
}

#[test]
fn an_empty_diff_says_so() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first")]);
    tui.keys("d");
    tui.send(Event::Review {
        session: id("first"),
        purpose: ReviewPurpose::BuiltIn,
        result: Ok(data(Vec::new())),
    });
    assert!(tui.screen().contains("no changes against main"));
}

#[test]
fn a_failed_diff_is_reported() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first")]);
    tui.keys("d");
    tui.send(Event::Review {
        session: id("first"),
        purpose: ReviewPurpose::BuiltIn,
        result: Err(orch_git::Error::Git {
            command: "git merge-base".into(),
            stderr: "fatal: bad revision".into(),
        }),
    });
    assert!(statusline(&mut tui).contains("fatal: bad revision"));
}

#[test]
fn capital_d_runs_the_external_review_on_the_same_change_set() {
    let mut tui = Harness::with_config(TuiConfig {
        review_command: "delta-review".into(),
        ..TuiConfig::default()
    });
    tui.sessions(vec![session("webshop", "first")]);
    tui.keys("D");
    assert!(tui.take_effects().contains(&Effect::LoadReview {
        session: id("first"),
        target: target(),
        purpose: ReviewPurpose::External,
    }));

    tui.send(Event::Review {
        session: id("first"),
        purpose: ReviewPurpose::External,
        result: Ok(data(Vec::new())),
    });
    let expected = Effect::RunExternal {
        command: "delta-review".into(),
        cwd: PathBuf::from("/home/me/webshop/.orchestrator/worktrees/first"),
        env: vec![
            ("ORCH_BASE".into(), "main".into()),
            ("ORCH_MERGE_BASE".into(), "b45e".into()),
            ("ORCH_REVIEW_TREE".into(), "7ree".into()),
        ],
    };
    assert!(tui.take_effects().contains(&expected));
    assert!(!tui.screen().contains("no changes against"));

    tui.command("review!");
    assert!(tui.take_effects().iter().any(|effect| matches!(
        effect,
        Effect::LoadReview {
            purpose: ReviewPurpose::External,
            ..
        }
    )));
    let _ = KeyCode::Null;
}

#[test]
fn the_default_external_review_diffs_the_merge_base_against_the_snapshot() {
    assert_eq!(
        TuiConfig::default().review_command,
        r#"git -p diff "$ORCH_MERGE_BASE" "$ORCH_REVIEW_TREE""#
    );
}
