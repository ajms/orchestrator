use std::path::PathBuf;
use std::process::Command;

use crossterm::event::KeyCode;
use orch_git::ENV_REDIRECTING_GIT;
use orch_tui::TuiConfig;
use ratatui::style::Color;
use tempfile::TempDir;

use crate::common::*;
use crate::new_session::{config, field, form};

fn row_of(tui: &mut Harness, needle: &str) -> usize {
    let lines = tui.lines();
    lines
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not in\n{}", lines.join("\n")))
}

fn with_repos(repos: &[&str]) -> Harness {
    let mut tui = Harness::with_config(TuiConfig {
        repos: repos.iter().map(PathBuf::from).collect(),
        ..TuiConfig::default()
    });
    tui.command("new");
    tui.ctrl('r');
    tui
}

#[test]
fn an_empty_filter_lists_known_repos_most_recent_first_with_the_current_one_highlighted() {
    let mut tui = Harness::with_config(config());
    tui.sessions(vec![
        session("older", "a"),
        session("older", "b"),
        session("third", "c"),
    ]);
    tui.command("new");
    tui.ctrl('r');

    let recent = row_of(&mut tui, "/home/me/recent");
    let older = row_of(&mut tui, "/home/me/older");
    let third = row_of(&mut tui, "/home/me/third");
    assert!(recent < older && older < third, "{}", tui.screen());
    assert!(tui.line_with("▸").contains("/home/me/older"));
    assert!(tui.line_with("/home/me/older").contains("2 live"));
    assert_eq!(tui.colour_of("2 live"), Color::DarkGray);
}

#[test]
fn plain_text_is_matched_fuzzily_and_ranked_by_name_start_name_path_then_subsequence() {
    let mut tui = with_repos(&[
        "/work/alpha-pie",
        "/home/api/web",
        "/home/me/xapi",
        "/home/me/apiserver",
        "/home/me/zzz",
        "/home/me/API-docs",
    ]);
    tui.keys("api");

    let rows: Vec<usize> = [
        "/home/me/apiserver",
        "/home/me/API-docs",
        "/home/me/xapi",
        "/home/api/web",
        "/work/alpha-pie",
    ]
    .iter()
    .map(|repo| row_of(&mut tui, repo))
    .collect();
    assert!(rows.is_sorted(), "{}", tui.screen());
    assert!(!tui.screen().contains("/home/me/zzz"));
    assert!(tui.line_with("▸").contains("/home/me/apiserver"));
}

struct Folders {
    dir: TempDir,
}

impl Folders {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn root(&self) -> String {
        self.dir
            .path()
            .canonicalize()
            .unwrap()
            .display()
            .to_string()
    }

    fn folder(&self, name: &str) -> PathBuf {
        let path = self.dir.path().canonicalize().unwrap().join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn repo(&self, name: &str) -> PathBuf {
        let path = self.folder(name);
        let mut command = Command::new("git");
        for key in ENV_REDIRECTING_GIT {
            command.env_remove(key);
        }
        let status = command
            .arg("-C")
            .arg(&path)
            .args(["init", "-q"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .unwrap();
        assert!(status.success());
        path
    }
}

fn picker_at(home: Option<&Folders>) -> Harness {
    let mut tui = Harness::with_config(TuiConfig {
        home: home.map(|home| PathBuf::from(home.root())),
        ..config()
    });
    tui.command("new");
    tui.ctrl('r');
    tui
}

#[test]
fn a_path_lists_its_subfolders_a_to_z_with_a_git_mark_and_this_folder_first() {
    let folders = Folders::new();
    folders.folder("zeta");
    folders.repo("beta");
    folders.folder("alpha");
    folders.folder(".hidden");
    std::fs::write(folders.dir.path().join("notes.txt"), "").unwrap();
    let mut tui = picker_at(None);
    tui.keys(&format!("{}/", folders.root()));

    let screen = tui.screen();
    assert!(tui.line_with("▸").contains("this folder"), "{screen}");
    let rows: Vec<usize> = ["this folder", "alpha/", "beta/", "zeta/"]
        .iter()
        .map(|name| row_of(&mut tui, name))
        .collect();
    assert!(rows.is_sorted(), "{screen}");
    assert!(tui.line_with("beta/").contains("git"), "{screen}");
    assert!(!tui.line_with("alpha/").contains("git"), "{screen}");
    assert!(!screen.contains("notes.txt"), "{screen}");
    assert!(!screen.contains(".hidden"), "{screen}");
}

#[test]
fn the_typed_segment_filters_the_folders_and_a_dot_shows_hidden_ones() {
    let folders = Folders::new();
    folders.folder("alpha");
    folders.folder("beta");
    folders.folder(".hidden");
    let mut tui = picker_at(None);
    tui.keys(&format!("{}/b", folders.root()));
    let screen = tui.screen();
    assert!(tui.line_with("▸").contains("beta/"), "{screen}");
    assert!(!screen.contains("alpha/"), "{screen}");
    assert!(!screen.contains("this folder"), "{screen}");

    tui.ctrl('u');
    tui.keys(&format!("{}/.", folders.root()));
    assert!(tui.line_with("▸").contains(".hidden/"), "{}", tui.screen());
}

#[test]
fn a_tilde_is_expanded_to_the_home_directory() {
    let home = Folders::new();
    home.folder("projects");
    let mut tui = picker_at(Some(&home));
    tui.keys("~/");
    assert!(tui.screen().contains("projects/"), "{}", tui.screen());

    tui.ctrl('u');
    tui.keys("~");
    tui.press(KeyCode::Down);
    tui.press(KeyCode::Tab);
    assert!(tui.screen().contains("> ~/projects/"), "{}", tui.screen());
}

#[test]
fn the_list_keys_move_the_highlight_and_the_list_scrolls_to_keep_it_visible() {
    let folders = Folders::new();
    for at in 0..205 {
        folders.folder(&format!("f{at:03}"));
    }
    let mut tui = picker_at(None);
    tui.keys(&format!("{}/", folders.root()));
    tui.ctrl('n');
    assert!(tui.line_with("▸").contains("f000/"));
    tui.press(KeyCode::Down);
    tui.ctrl('p');
    tui.ctrl('p');
    assert!(tui.line_with("▸").contains("this folder"));

    for _ in 0..30 {
        tui.press(KeyCode::PageDown);
    }
    let screen = tui.screen();
    assert!(tui.line_with("▸").contains("f199/"), "{screen}");
    assert!(screen.contains("… 5 more"), "{screen}");
    assert!(!screen.contains("f000/"), "{screen}");
    assert!(!screen.contains("f200/"), "{screen}");

    for _ in 0..30 {
        tui.press(KeyCode::PageUp);
    }
    assert!(tui.line_with("▸").contains("this folder"));
    assert!(tui.screen().contains("f000/"));
}

#[test]
fn tab_in_path_mode_completes_the_highlighted_folder_and_lists_inside_it() {
    let folders = Folders::new();
    folders.folder("alpha/inner");
    let mut tui = picker_at(None);
    tui.keys(&format!("{}/al", folders.root()));
    tui.press(KeyCode::Tab);
    let query = format!("> {}/alpha/", folders.root());
    assert!(tui.screen().contains(&query), "{}", tui.screen());
    assert!(tui.screen().contains("inner/"), "{}", tui.screen());
}

#[test]
fn tab_in_filter_mode_switches_to_the_highlighted_repos_path() {
    let mut tui = picker_at(None);
    tui.keys("older");
    tui.press(KeyCode::Tab);
    assert!(
        tui.screen().contains("> /home/me/older/"),
        "{}",
        tui.screen()
    );
}

fn picked_repo(tui: &mut Harness) -> PathBuf {
    tui.keys(" it");
    tui.ctrl('s');
    create_requests(tui).pop().expect("no Session created").repo
}

#[test]
fn picking_a_folder_inside_a_repo_resolves_to_its_git_root() {
    let home = Folders::new();
    let root = home.repo("repos/x");
    home.folder("repos/x/src");
    let mut tui = picker_at(Some(&home));
    tui.keys("~/repos/x/src/");
    tui.press(KeyCode::Enter);

    assert!(
        field(&mut tui, "Repo").contains("→ ~/repos/x"),
        "{}",
        tui.screen()
    );
    assert_eq!(picked_repo(&mut tui), root);
}

#[test]
fn picking_a_repo_root_by_path_uses_it_as_is() {
    let folders = Folders::new();
    let root = folders.repo("x");
    let mut tui = picker_at(None);
    tui.keys(&format!("{}/", folders.root()));
    tui.press(KeyCode::Down);
    tui.press(KeyCode::Enter);

    let repo = field(&mut tui, "Repo");
    assert!(repo.contains(&root.display().to_string()), "{repo}");
    assert!(!repo.contains('→'), "{repo}");
    assert_eq!(picked_repo(&mut tui), root);
}

#[test]
fn a_path_in_no_git_repo_is_refused_and_the_picker_stays_open() {
    let folders = Folders::new();
    folders.folder("plain");
    let mut tui = picker_at(None);
    tui.keys(&format!("{}/plain/", folders.root()));
    tui.press(KeyCode::Enter);

    let screen = tui.screen();
    assert!(screen.contains("not a git repository"), "{screen}");
    assert!(
        screen.contains(&format!("> {}/plain/", folders.root())),
        "{screen}"
    );
}

#[test]
fn a_typed_path_that_does_not_exist_is_refused_rather_than_used() {
    let mut tui = picker_at(None);
    tui.keys("/no/such/place");
    tui.press(KeyCode::Enter);
    assert!(tui.screen().contains("not a git repository"));
    tui.press(KeyCode::Esc);
    assert_eq!(picked_repo(&mut tui), PathBuf::from("/home/me/recent"));
}

#[test]
fn picking_from_the_repo_field_returns_focus_to_the_prompt() {
    let mut tui = form(config());
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Enter);
    tui.press(KeyCode::Down);
    tui.press(KeyCode::Enter);
    tui.keys("typed");
    tui.ctrl('s');
    let create = create_requests(&mut tui).pop().expect("no Session created");
    assert_eq!(create.prompt, "typed");
    assert_eq!(create.repo, PathBuf::from("/home/me/older"));
}

#[test]
fn relative_paths_are_ordinary_filter_text() {
    let mut tui = with_repos(&["/home/me/recent"]);
    tui.keys("../");
    let screen = tui.screen();
    assert!(screen.contains("no matching Repo"), "{screen}");
    assert!(!screen.contains("this folder"), "{screen}");
}

#[test]
fn a_filter_matching_nothing_hints_at_path_mode() {
    let mut tui = with_repos(&["/home/me/recent"]);
    tui.keys("nothing");
    assert!(
        tui.screen()
            .contains("no matching Repo — start with / or ~ for a path")
    );
}
