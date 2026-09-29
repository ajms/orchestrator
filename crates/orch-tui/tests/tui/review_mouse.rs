use std::path::PathBuf;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
use orch_tui::{Effect, Event, FileDiff, ReviewData, ReviewPurpose};
use ratatui::style::Color;

use crate::common::*;

const LIST_LEFT: u16 = 1;
const DIFF_LEFT: u16 = SIDEBAR + 1;
const TOP: u16 = 1;

fn login() -> FileDiff {
    FileDiff {
        path: "src/login.rs".into(),
        lines: vec![
            "@@ -1,3 +1,3 @@".into(),
            " use std::time::Duration;".into(),
            "-let timeout = 50;".into(),
            "+let timeout = config.login_timeout();".into(),
            "+log::debug!(\"timeout\");".into(),
            "@@ -10,2 +11,3 @@".into(),
            " fn main() {".into(),
            "+    run();".into(),
            " }".into(),
        ],
        deleted: false,
    }
}

fn config() -> FileDiff {
    FileDiff {
        path: "src/config.rs".into(),
        lines: vec!["@@ -0,0 +1 @@".into(), "+pub struct Config;".into()],
        deleted: false,
    }
}

fn reviewing(files: Vec<FileDiff>) -> Harness {
    let mut tui = Harness::new();
    tui.sessions(vec![
        session("webshop", "first"),
        session("webshop", "second"),
    ]);
    tui.keys("d");
    tui.send(Event::Review {
        session: id("first"),
        purpose: ReviewPurpose::BuiltIn,
        result: Ok(ReviewData {
            merge_base: "b45e".into(),
            tree: "7ree".into(),
            files,
        }),
    });
    tui.take_effects();
    tui
}

fn many_files() -> Vec<FileDiff> {
    let long = FileDiff {
        path: "src/f00.rs".into(),
        lines: std::iter::once("@@ -1,60 +1,60 @@".to_string())
            .chain((1..=60).map(|n| format!(" body {n:02}")))
            .collect(),
        deleted: false,
    };
    let rest = (1..40).map(|n| FileDiff {
        path: format!("src/f{n:02}.rs"),
        lines: vec!["@@ -0,0 +1 @@".into(), "+x".into()],
        deleted: false,
    });
    std::iter::once(long).chain(rest).collect()
}

fn in_diff(col: u16, row: u16) -> (u16, u16) {
    (DIFF_LEFT + col, TOP + row)
}

fn click_diff(tui: &mut Harness, col: u16, row: u16) {
    let (x, y) = in_diff(col, row);
    tui.click(x, y);
}

fn ctrl_click_diff(tui: &mut Harness, row: u16) {
    let (x, y) = in_diff(3, row);
    tui.mouse(
        MouseEventKind::Down(MouseButton::Left),
        x,
        y,
        KeyModifiers::CONTROL,
    );
    tui.mouse(
        MouseEventKind::Up(MouseButton::Left),
        x,
        y,
        KeyModifiers::CONTROL,
    );
}

fn select_diff(tui: &mut Harness, from: (u16, u16), to: (u16, u16)) {
    let (x, y) = in_diff(from.0, from.1);
    tui.mouse_down(x, y);
    let (x, y) = in_diff(to.0, to.1);
    tui.drag(x, y);
    tui.release(x, y);
}

fn copies(tui: &mut Harness) -> Vec<String> {
    tui.take_effects()
        .iter()
        .filter_map(|effect| match effect {
            Effect::WriteTerminal(bytes) => {
                let text = String::from_utf8(bytes.clone()).ok()?;
                let payload = text.strip_prefix("\x1b]52;c;")?.strip_suffix('\x07')?;
                String::from_utf8(STANDARD.decode(payload).ok()?).ok()
            }
            _ => None,
        })
        .collect()
}

fn last_copy(tui: &mut Harness) -> Option<String> {
    copies(tui).pop()
}

fn editor_opens(tui: &mut Harness) -> Vec<(PathBuf, Option<u32>, PathBuf)> {
    tui.take_effects()
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::OpenInEditor { file, line, cwd } => Some((file, line, cwd)),
            _ => None,
        })
        .collect()
}

fn worktree() -> PathBuf {
    PathBuf::from("/home/me/webshop/.orchestrator/worktrees/first")
}

fn opened_at(file: &str, line: Option<u32>) -> Vec<(PathBuf, Option<u32>, PathBuf)> {
    vec![(PathBuf::from(file), line, worktree())]
}

fn diff_top_line(tui: &mut Harness) -> String {
    tui.lines()[usize::from(TOP)]
        .chars()
        .skip(usize::from(DIFF_LEFT))
        .collect()
}

fn list_top_line(tui: &mut Harness) -> String {
    tui.lines()[usize::from(TOP)]
        .chars()
        .take(usize::from(SIDEBAR))
        .collect()
}

#[test]
fn the_wheel_scrolls_the_file_list_and_the_diff_independently() {
    let mut tui = reviewing(many_files());
    assert!(list_top_line(&mut tui).contains("src/f00.rs"));
    assert!(diff_top_line(&mut tui).contains("@@ -1,60"));

    tui.wheel(MouseEventKind::ScrollDown, LIST_LEFT + 3, TOP + 5);
    assert!(list_top_line(&mut tui).contains("src/f03.rs"));
    assert!(diff_top_line(&mut tui).contains("@@ -1,60"));

    let (x, y) = in_diff(5, 5);
    tui.wheel(MouseEventKind::ScrollDown, x, y);
    tui.wheel(MouseEventKind::ScrollDown, x, y);
    assert!(diff_top_line(&mut tui).contains(" body 06"));
    assert!(list_top_line(&mut tui).contains("src/f03.rs"));

    tui.wheel(MouseEventKind::ScrollUp, x, y);
    assert!(diff_top_line(&mut tui).contains(" body 03"));
    tui.wheel(MouseEventKind::ScrollUp, LIST_LEFT + 3, TOP + 5);
    assert!(list_top_line(&mut tui).contains("src/f00.rs"));
}

#[test]
fn clicking_a_file_shows_its_diff() {
    let mut tui = reviewing(vec![login(), config()]);
    tui.click(LIST_LEFT + 3, TOP + 1);

    let screen = tui.screen();
    assert!(screen.contains("+pub struct Config;"), "{screen}");
    assert!(!screen.contains("+let timeout"), "{screen}");
}

#[test]
fn a_drag_in_the_diff_copies_the_lines_as_shown_with_their_markers() {
    let mut tui = reviewing(vec![login()]);
    select_diff(&mut tui, (0, 1), (8, 3));

    assert_eq!(
        last_copy(&mut tui).as_deref(),
        Some(" use std::time::Duration;\n-let timeout = 50;\n+let time")
    );
}

#[test]
fn the_diff_selection_is_highlighted_like_a_pane_selection_and_a_click_clears_it() {
    let mut tui = reviewing(vec![login()]);
    select_diff(&mut tui, (0, 2), (4, 2));
    assert_eq!(tui.background_of("-let t"), Color::Rgb(70, 70, 110));

    tui.later(1000);
    click_diff(&mut tui, 10, 6);
    assert_eq!(tui.background_of("-let t"), Color::Reset);
}

#[test]
fn a_double_click_in_the_diff_selects_a_word() {
    let mut tui = reviewing(vec![login()]);
    click_diff(&mut tui, 6, 2);
    click_diff(&mut tui, 6, 2);

    assert_eq!(last_copy(&mut tui).as_deref(), Some("timeout"));
}

#[test]
fn a_triple_click_in_the_diff_selects_the_line() {
    let mut tui = reviewing(vec![login()]);
    click_diff(&mut tui, 6, 3);
    click_diff(&mut tui, 6, 3);
    click_diff(&mut tui, 6, 3);

    assert_eq!(
        last_copy(&mut tui).as_deref(),
        Some("+let timeout = config.login_timeout();")
    );
}

#[test]
fn dragging_past_the_bottom_of_the_diff_auto_scrolls() {
    let mut tui = reviewing(many_files());
    let (x, y) = in_diff(1, 25);
    tui.mouse_down(x, y);
    let (x, _) = in_diff(8, 0);
    tui.drag(x, TOP + 30);
    tui.tick();
    tui.tick();
    tui.release(x, TOP + 30);

    let copied = last_copy(&mut tui).unwrap();
    assert!(copied.starts_with("body 25\n body 26"), "{copied}");
    assert!(copied.ends_with("\n body 29"), "{copied}");
    assert!(diff_top_line(&mut tui).contains(" body 03"));
}

#[test]
fn ctrl_click_opens_the_new_file_line_of_an_added_line() {
    let mut tui = reviewing(vec![login()]);
    ctrl_click_diff(&mut tui, 3);
    assert_eq!(editor_opens(&mut tui), opened_at("src/login.rs", Some(2)));

    ctrl_click_diff(&mut tui, 7);
    assert_eq!(editor_opens(&mut tui), opened_at("src/login.rs", Some(12)));
}

#[test]
fn ctrl_click_opens_the_new_file_line_of_a_context_line() {
    let mut tui = reviewing(vec![login()]);
    ctrl_click_diff(&mut tui, 1);
    assert_eq!(editor_opens(&mut tui), opened_at("src/login.rs", Some(1)));

    ctrl_click_diff(&mut tui, 8);
    assert_eq!(editor_opens(&mut tui), opened_at("src/login.rs", Some(13)));
}

#[test]
fn ctrl_click_on_a_removed_line_opens_the_new_line_now_in_its_place() {
    let mut tui = reviewing(vec![login()]);
    ctrl_click_diff(&mut tui, 2);

    assert_eq!(editor_opens(&mut tui), opened_at("src/login.rs", Some(2)));
}

#[test]
fn ctrl_click_on_a_hunk_header_opens_the_hunks_first_new_line() {
    let mut tui = reviewing(vec![login()]);
    ctrl_click_diff(&mut tui, 5);

    assert_eq!(editor_opens(&mut tui), opened_at("src/login.rs", Some(11)));
    assert_eq!(copies(&mut tui), Vec::<String>::new());
}

#[test]
fn ctrl_click_on_a_file_in_the_list_only_shows_it() {
    let mut tui = reviewing(vec![login(), config()]);
    tui.mouse(
        MouseEventKind::Down(MouseButton::Left),
        LIST_LEFT + 3,
        TOP + 1,
        KeyModifiers::CONTROL,
    );

    assert_eq!(editor_opens(&mut tui), Vec::new());
    assert!(tui.screen().contains("+pub struct Config;"));
}

#[test]
fn the_editor_opens_in_the_reviewed_sessions_worktree_after_the_selection_moved() {
    let mut tui = Harness::new();
    tui.sessions(vec![
        session("webshop", "first"),
        session("webshop", "second"),
    ]);
    tui.keys("d");
    tui.keys("j");
    tui.send(Event::Review {
        session: id("first"),
        purpose: ReviewPurpose::BuiltIn,
        result: Ok(ReviewData {
            merge_base: "b45e".into(),
            tree: "7ree".into(),
            files: vec![login()],
        }),
    });
    tui.take_effects();

    ctrl_click_diff(&mut tui, 1);
    assert_eq!(editor_opens(&mut tui), opened_at("src/login.rs", Some(1)));
    tui.keys("o");
    assert_eq!(editor_opens(&mut tui), opened_at("src/login.rs", Some(1)));
}

#[test]
fn a_removed_file_is_not_opened_but_reported() {
    let mut tui = reviewing(vec![FileDiff {
        path: "src/old.rs".into(),
        lines: vec!["@@ -1,2 +0,0 @@".into(), "-a".into(), "-b".into()],
        deleted: true,
    }]);
    ctrl_click_diff(&mut tui, 1);
    assert_eq!(editor_opens(&mut tui), Vec::new());
    assert!(statusline(&mut tui).contains("src/old.rs was deleted"));

    tui.keys("o");
    assert_eq!(editor_opens(&mut tui), Vec::new());
    assert!(statusline(&mut tui).contains("src/old.rs was deleted"));
}

#[test]
fn ctrl_d_and_ctrl_u_stop_where_the_wheel_stops() {
    let mut tui = reviewing(many_files());
    for _ in 0..4 {
        tui.ctrl('d');
    }
    assert!(diff_top_line(&mut tui).contains(" body 34"));
    tui.ctrl('u');
    assert!(diff_top_line(&mut tui).contains(" body 24"));
}

#[test]
fn o_opens_the_current_files_first_hunk() {
    let mut tui = reviewing(vec![login(), config()]);
    tui.keys("o");
    assert_eq!(editor_opens(&mut tui), opened_at("src/login.rs", Some(1)));

    tui.keys("j");
    tui.keys("o");
    assert_eq!(editor_opens(&mut tui), opened_at("src/config.rs", Some(1)));
}

#[test]
fn o_opens_a_file_without_hunks_without_a_line() {
    let mut tui = reviewing(vec![FileDiff {
        path: "assets/logo.png".into(),
        lines: Vec::new(),
        deleted: false,
    }]);
    tui.keys("o");

    assert_eq!(editor_opens(&mut tui), opened_at("assets/logo.png", None));
}

#[test]
fn a_click_where_the_sidebar_would_be_leaves_the_selected_session_alone() {
    let mut tui = reviewing(vec![login()]);
    let before = tui.daemon().last_view();
    let second_row = tui
        .sidebar_lines()
        .iter()
        .position(|line| line.contains("src/login.rs"))
        .unwrap() as u16
        + 2;
    tui.click(LIST_LEFT + 3, second_row);
    tui.keys("q");

    assert_eq!(tui.daemon().last_view(), before);
    assert!(tui.line_with("first").contains("webshop / first"));
}

#[test]
fn the_mouse_in_the_review_keeps_normal_mode() {
    let mut tui = reviewing(vec![login(), config()]);
    select_diff(&mut tui, (0, 1), (4, 1));
    ctrl_click_diff(&mut tui, 1);
    tui.click(LIST_LEFT + 3, TOP + 1);
    tui.wheel(MouseEventKind::ScrollDown, LIST_LEFT + 3, TOP + 1);

    assert!(statusline(&mut tui).contains(" NORMAL "));
}

#[test]
fn a_drag_over_wide_characters_copies_them_once() {
    let mut tui = reviewing(vec![FileDiff {
        path: "src/names.rs".into(),
        lines: vec!["@@ -0,0 +1 @@".into(), "+let 名前 = 1;".into()],
        deleted: false,
    }]);
    select_diff(&mut tui, (5, 1), (8, 1));

    assert_eq!(last_copy(&mut tui).as_deref(), Some("名前"));
}

#[test]
fn a_drag_copies_long_lines_only_as_far_as_the_column_shows_them() {
    let long = format!("+{}", "x".repeat(100));
    let mut tui = reviewing(vec![FileDiff {
        path: "src/long.rs".into(),
        lines: vec!["@@ -0,0 +1,2 @@".into(), long, "+y".into()],
        deleted: false,
    }]);
    select_diff(&mut tui, (0, 1), (1, 2));

    let shown = format!("+{}", "x".repeat(usize::from(WIDTH - DIFF_LEFT - 2)));
    assert_eq!(last_copy(&mut tui), Some(format!("{shown}\n+y")));
}
