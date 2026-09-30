use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use orch_protocol::FromDaemon;
use orch_tui::Effect;
use ratatui::style::Modifier;

use crate::common::*;

const LEFT: MouseEventKind = MouseEventKind::Down(MouseButton::Left);
const MOVED: MouseEventKind = MouseEventKind::Moved;

fn pane_in(worktree: &Path, text: &str) -> Harness {
    let mut tui = Harness::new();
    let mut view = session("webshop", "first");
    view.worktree = worktree.to_path_buf();
    tui.sessions(vec![view]);
    output(&mut tui, text);
    tui.take_effects();
    tui
}

fn pane_with(text: &str) -> Harness {
    pane_in(Path::new("/nonexistent/worktree"), text)
}

fn output(tui: &mut Harness, text: &str) {
    let bytes = text.as_bytes().to_vec();
    tui.pane("first", FromDaemon::Output { bytes });
}

fn at(tui: &mut Harness, kind: MouseEventKind, col: u16, row: u16, mods: KeyModifiers) {
    let (x, y) = in_pane(col, row);
    tui.mouse(kind, x, y, mods);
}

fn ctrl_click(tui: &mut Harness, col: u16, row: u16) {
    at(tui, LEFT, col, row, KeyModifiers::CONTROL);
    let up = MouseEventKind::Up(MouseButton::Left);
    at(tui, up, col, row, KeyModifiers::CONTROL);
}

fn hover(tui: &mut Harness, col: u16, row: u16, mods: KeyModifiers) {
    at(tui, MOVED, col, row, mods);
}

fn opened_urls(tui: &mut Harness) -> Vec<String> {
    tui.take_effects()
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::OpenUrl { url } => Some(url),
            _ => None,
        })
        .collect()
}

fn opened_files(tui: &mut Harness) -> Vec<(PathBuf, Option<u32>, PathBuf)> {
    tui.take_effects()
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::OpenInEditor { file, line, cwd } => Some((file, line, cwd)),
            _ => None,
        })
        .collect()
}

fn opens_anything(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::OpenUrl { .. } | Effect::OpenInEditor { .. }))
}

fn underlined(tui: &mut Harness, col: u16, row: u16) -> bool {
    tui.draw();
    let (x, y) = in_pane(col, row);
    let cell = &tui.terminal.backend().buffer()[(x, y)];
    cell.modifier.contains(Modifier::UNDERLINED)
}

fn worktree_with(file: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "fn main() {}\n").unwrap();
    dir
}

#[test]
fn ctrl_click_on_a_url_opens_it_with_xdg_open() {
    let mut tui = pane_with("see https://example.com/docs for more");
    ctrl_click(&mut tui, 10, 0);

    assert_eq!(opened_urls(&mut tui), vec!["https://example.com/docs"]);
}

#[test]
fn ctrl_click_opens_the_whole_url_wrapped_across_rows() {
    let url = format!("https://example.com/{}", "a".repeat(100));
    let mut tui = pane_with(&format!("x {url} y"));
    ctrl_click(&mut tui, 3, 1);

    assert_eq!(opened_urls(&mut tui), vec![url]);
}

#[test]
fn trailing_punctuation_is_not_part_of_the_url() {
    let mut tui = pane_with("visit https://example.com/a, then (https://example.com/b_(c)).");
    ctrl_click(&mut tui, 8, 0);
    ctrl_click(&mut tui, 40, 0);

    assert_eq!(
        opened_urls(&mut tui),
        vec!["https://example.com/a", "https://example.com/b_(c)"]
    );
}

#[test]
fn http_and_file_urls_open_too() {
    let mut tui = pane_with("http://localhost:8080/x file:///tmp/report.html");
    ctrl_click(&mut tui, 2, 0);
    ctrl_click(&mut tui, 30, 0);

    assert_eq!(
        opened_urls(&mut tui),
        vec!["http://localhost:8080/x", "file:///tmp/report.html"]
    );
}

#[test]
fn ctrl_click_on_an_existing_relative_path_opens_it_in_the_editor_at_the_line() {
    let dir = worktree_with("src/main.rs");
    let mut tui = pane_in(dir.path(), "error at src/main.rs:12: oops");
    ctrl_click(&mut tui, 12, 0);

    let file = dir.path().join("src/main.rs");
    assert_eq!(
        opened_files(&mut tui),
        vec![(file, Some(12), dir.path().to_path_buf())]
    );
}

#[test]
fn a_path_with_line_and_column_opens_at_the_line() {
    let dir = worktree_with("src/lib.rs");
    let mut tui = pane_in(dir.path(), "  --> src/lib.rs:7:5");
    ctrl_click(&mut tui, 8, 0);

    let opened = opened_files(&mut tui);
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].1, Some(7));
}

#[test]
fn a_path_without_a_line_opens_the_file() {
    let dir = worktree_with("README.md");
    let mut tui = pane_in(dir.path(), "edited README.md.");
    ctrl_click(&mut tui, 9, 0);

    let file = dir.path().join("README.md");
    assert_eq!(
        opened_files(&mut tui),
        vec![(file, None, dir.path().to_path_buf())]
    );
}

#[test]
fn an_absolute_path_opens_wherever_it_is() {
    let dir = worktree_with("notes.txt");
    let file = dir.path().join("notes.txt");
    let mut tui = pane_with(&format!("wrote {}:3", file.display()));
    ctrl_click(&mut tui, 8, 0);

    let opened = opened_files(&mut tui);
    assert_eq!(opened.len(), 1);
    assert_eq!((opened[0].0.clone(), opened[0].1), (file, Some(3)));
}

#[test]
fn ctrl_click_on_a_path_that_does_not_exist_opens_nothing() {
    let dir = worktree_with("src/main.rs");
    let mut tui = pane_in(dir.path(), "error at src/missing.rs:12");
    ctrl_click(&mut tui, 12, 0);

    assert!(!opens_anything(&tui.take_effects()));
}

#[test]
fn a_plain_click_on_a_url_opens_nothing_and_a_double_click_selects_it() {
    let mut tui = pane_with("see https://example.com/docs for more");
    let (x, y) = in_pane(10, 0);
    tui.click(x, y);
    assert!(!opens_anything(&tui.take_effects()));

    tui.click(x, y);
    let effects = tui.take_effects();
    assert!(!opens_anything(&effects));
    assert!(!effects.is_empty(), "the double-click copies the word");
}

#[test]
fn ctrl_click_on_a_link_keeps_the_pane_selection() {
    let mut tui = pane_with("hello https://example.com/docs");
    let before = tui.background_of("https");
    let (x, y) = in_pane(0, 0);
    tui.mouse_down(x, y);
    tui.drag(x + 3, y);
    tui.release(x + 3, y);
    let selected = tui.background_of("hello");
    assert_ne!(selected, before);
    tui.take_effects();
    tui.later(1000);

    ctrl_click(&mut tui, 10, 0);

    let effects = tui.take_effects();
    assert!(opens_anything(&effects));
    assert_eq!(effects.len(), 1, "no copy: {effects:?}");
    assert_eq!(tui.background_of("hello"), selected);
}

#[test]
fn ctrl_drag_off_a_link_selects_like_a_plain_drag() {
    let mut tui = pane_with("hello world");
    at(&mut tui, LEFT, 0, 0, KeyModifiers::CONTROL);
    let drag = MouseEventKind::Drag(MouseButton::Left);
    at(&mut tui, drag, 4, 0, KeyModifiers::CONTROL);
    let up = MouseEventKind::Up(MouseButton::Left);
    at(&mut tui, up, 4, 0, KeyModifiers::CONTROL);

    let effects = tui.take_effects();
    assert!(!effects.is_empty(), "the drag copies");
    assert!(!opens_anything(&effects));
}

#[test]
fn holding_ctrl_over_a_link_underlines_it() {
    let mut tui = pane_with("see https://example.com/docs for more");
    hover(&mut tui, 10, 0, KeyModifiers::CONTROL);

    assert!(underlined(&mut tui, 4, 0));
    assert!(underlined(&mut tui, 27, 0));
    assert!(!underlined(&mut tui, 3, 0));
    assert!(!underlined(&mut tui, 28, 0));
}

#[test]
fn the_underline_covers_a_url_wrapped_across_rows() {
    let url = format!("https://example.com/{}", "a".repeat(100));
    let mut tui = pane_with(&format!("x {url} y"));
    hover(&mut tui, 3, 1, KeyModifiers::CONTROL);

    assert!(underlined(&mut tui, 2, 0));
    assert!(underlined(&mut tui, 77, 0));
    assert!(underlined(&mut tui, 43, 1));
    assert!(!underlined(&mut tui, 44, 1));
}

#[test]
fn the_underline_clears_without_ctrl_or_off_the_link() {
    let mut tui = pane_with("see https://example.com/docs for more");
    hover(&mut tui, 10, 0, KeyModifiers::CONTROL);
    hover(&mut tui, 10, 0, KeyModifiers::NONE);
    assert!(!underlined(&mut tui, 10, 0));

    hover(&mut tui, 10, 0, KeyModifiers::CONTROL);
    hover(&mut tui, 32, 0, KeyModifiers::CONTROL);
    assert!(!underlined(&mut tui, 10, 0));

    hover(&mut tui, 10, 0, KeyModifiers::CONTROL);
    tui.mouse(MOVED, 5, 5, KeyModifiers::CONTROL);
    assert!(!underlined(&mut tui, 10, 0));
}

#[test]
fn a_plain_word_does_not_underline() {
    let mut tui = pane_with("see https://example.com/docs for more");
    hover(&mut tui, 30, 0, KeyModifiers::CONTROL);

    assert!(!underlined(&mut tui, 30, 0));
}

#[test]
fn with_mouse_passthrough_ctrl_click_goes_to_the_agent() {
    let mut tui = pane_with("\x1b[?1000h\x1b[?1006hsee https://example.com/docs");
    tui.daemon().input.clear();
    ctrl_click(&mut tui, 10, 0);

    assert!(!opens_anything(&tui.take_effects()));
    let sent = String::from_utf8_lossy(&tui.daemon().input).into_owned();
    assert_eq!(sent, "\x1b[<16;11;1M\x1b[<16;11;1m");
}

#[test]
fn a_selection_after_a_ctrl_press_whose_release_went_missing_still_copies() {
    let dir = worktree_with("src/main.rs");
    let mut tui = pane_in(dir.path(), "error at src/main.rs:12 and more text");
    at(&mut tui, LEFT, 12, 0, KeyModifiers::CONTROL);
    tui.take_effects();
    tui.later(1000);

    let (x, y) = in_pane(28, 0);
    tui.mouse_down(x, y);
    tui.drag(x + 3, y);
    tui.release(x + 3, y);

    assert!(!tui.take_effects().is_empty(), "the drag copies");
}

#[test]
fn any_key_clears_the_underline() {
    let mut tui = pane_with("see https://example.com/docs for more");
    hover(&mut tui, 10, 0, KeyModifiers::CONTROL);
    tui.press(KeyCode::Esc);

    assert!(!underlined(&mut tui, 10, 0));
}

#[test]
fn output_redrawn_in_place_clears_the_underline() {
    let mut tui = pane_with("see https://example.com/docs for more");
    hover(&mut tui, 10, 0, KeyModifiers::CONTROL);
    output(&mut tui, "\x1b[Hsee plain text now");

    assert!(!underlined(&mut tui, 10, 0));
}
