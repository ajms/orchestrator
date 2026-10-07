use crossterm::event::{Event as TermEvent, KeyCode, KeyEvent, KeyModifiers};
use orch_tui::{Effect, Event};
use ratatui::style::Color;

use crate::common::*;
use crate::new_session::{config, field, form, form_with_presets};

fn shift_tab(tui: &mut Harness) {
    tui.key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
}

fn prompt_of_create(tui: &mut Harness) -> String {
    tui.ctrl('s');
    create_requests(tui)
        .pop()
        .expect("no Session created")
        .prompt
}

#[test]
fn tab_goes_from_the_prompt_through_repo_branch_base_agent_and_preset_back_to_the_prompt() {
    let mut tui = form_with_presets(config());
    tui.keys("Fix");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.keys("-x");
    assert!(field(&mut tui, "Branch").contains("orch/fix-x"));

    tui.press(KeyCode::Tab);
    tui.keys("main");
    assert!(field(&mut tui, "Base").contains("main"));

    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Right);
    assert!(field(&mut tui, "Preset").contains("plan"));

    tui.press(KeyCode::Tab);
    tui.keys(" it");
    assert_eq!(prompt_of_create(&mut tui), "Fix it");
}

#[test]
fn shift_tab_goes_back_from_the_prompt_to_the_preset() {
    let mut tui = form_with_presets(config());
    shift_tab(&mut tui);
    tui.press(KeyCode::Right);
    assert!(field(&mut tui, "Preset").contains("plan"));
}

#[test]
fn enter_on_branch_base_agent_and_preset_moves_to_the_next_field() {
    let mut tui = form_with_presets(config());
    tui.keys("Fix");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Enter);
    tui.keys("main");
    tui.press(KeyCode::Enter);
    tui.press(KeyCode::Enter);
    tui.press(KeyCode::Right);
    tui.press(KeyCode::Enter);
    tui.keys(" it");

    assert!(create_requests(&mut tui).is_empty());
    tui.ctrl('s');
    let create = create_requests(&mut tui).pop().unwrap();
    assert_eq!(create.prompt, "Fix it");
    assert_eq!(create.base.as_deref(), Some("main"));
    assert_eq!(create.preset.as_deref(), Some("plan"));
}

fn with(tui: &mut Harness, code: KeyCode, modifiers: KeyModifiers) {
    tui.key(KeyEvent::new(code, modifiers));
}

fn alt(tui: &mut Harness, c: char) {
    with(tui, KeyCode::Char(c), KeyModifiers::ALT);
}

fn typed(text: &str, edit: impl FnOnce(&mut Harness)) -> String {
    let mut tui = form(config());
    tui.keys(text);
    edit(&mut tui);
    prompt_of_create(&mut tui)
}

#[test]
fn left_and_right_move_the_cursor_by_character_for_inserting() {
    let prompt = typed("helo", |tui| {
        tui.press(KeyCode::Left);
        tui.press(KeyCode::Left);
        tui.press(KeyCode::Right);
        tui.keys("l");
    });
    assert_eq!(prompt, "hello");
}

#[test]
fn home_end_and_ctrl_a_e_jump_to_the_line_start_and_end() {
    let prompt = typed("first\nmiddle", |tui| {
        tui.press(KeyCode::Home);
        tui.keys("[");
        tui.press(KeyCode::End);
        tui.keys("]");
        tui.ctrl('a');
        tui.keys("<");
        tui.ctrl('e');
        tui.keys(">");
    });
    assert_eq!(prompt, "first\n<[middle]>");
}

#[test]
fn ctrl_arrows_and_alt_b_f_move_by_word() {
    let prompt = typed("one two three", |tui| {
        with(tui, KeyCode::Left, KeyModifiers::CONTROL);
        tui.keys("x");
        alt(tui, 'b');
        alt(tui, 'b');
        tui.keys("y");
        with(tui, KeyCode::Right, KeyModifiers::CONTROL);
        tui.keys("z");
        alt(tui, 'f');
        tui.keys("!");
    });
    assert_eq!(prompt, "one ytwoz xthree!");
}

#[test]
fn backspace_and_delete_remove_around_the_cursor() {
    let prompt = typed("abcd", |tui| {
        tui.press(KeyCode::Left);
        tui.press(KeyCode::Left);
        tui.press(KeyCode::Backspace);
        tui.press(KeyCode::Delete);
    });
    assert_eq!(prompt, "ad");
}

#[test]
fn ctrl_w_deletes_the_word_before_the_cursor() {
    let prompt = typed("fix the  bug", |tui| tui.ctrl('w'));
    assert_eq!(prompt, "fix the  ");
    let prompt = typed("fix the  ", |tui| tui.ctrl('w'));
    assert_eq!(prompt, "fix ");
}

#[test]
fn ctrl_u_and_ctrl_k_delete_to_the_line_start_and_end() {
    let prompt = typed("keep\nfirst second\nlast", |tui| {
        tui.press(KeyCode::Up);
        tui.press(KeyCode::End);
        alt(tui, 'b');
        tui.press(KeyCode::Left);
        tui.ctrl('k');
        tui.press(KeyCode::Left);
        tui.press(KeyCode::Left);
        tui.ctrl('u');
        tui.keys("only");
    });
    assert_eq!(prompt, "keep\nonlyst\nlast");
}

#[test]
fn up_and_down_move_between_prompt_lines_and_jump_to_the_ends_at_the_edges() {
    let prompt = typed("abc\nde\nfghij", |tui| {
        tui.press(KeyCode::Left);
        tui.press(KeyCode::Up);
        tui.keys("1");
        tui.press(KeyCode::Up);
        tui.keys("2");
        tui.press(KeyCode::Up);
        tui.keys("3");
        tui.press(KeyCode::Down);
        tui.press(KeyCode::Down);
        tui.press(KeyCode::Down);
        tui.keys("4");
    });
    assert_eq!(prompt, "3abc2\nde1\nfghij4");
}

#[test]
fn branch_and_base_have_the_same_editing_keys() {
    let mut tui = form(config());
    tui.keys("Fix");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Home);
    with(&mut tui, KeyCode::Right, KeyModifiers::CONTROL);
    tui.keys("-x");
    tui.press(KeyCode::Tab);
    tui.keys("release 1.2");
    tui.ctrl('w');
    tui.keys("main");
    tui.ctrl('s');
    let create = create_requests(&mut tui).pop().unwrap();
    assert_eq!(create.branch.as_deref(), Some("orch-x/fix"));
    assert_eq!(create.base.as_deref(), Some("release main"));
}

#[test]
fn clearing_the_branch_hands_it_back_to_the_prompt_derived_name_shown_dimmed() {
    let mut tui = form(config());
    tui.keys("Fix it");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.keys("-now");
    assert_ne!(tui.colour_of("orch/fix-it-now"), Color::DarkGray);

    tui.ctrl('u');
    assert_eq!(tui.colour_of("orch/fix-it"), Color::DarkGray);
    tui.press(KeyCode::BackTab);
    tui.press(KeyCode::BackTab);
    tui.keys(" today");
    assert!(field(&mut tui, "Branch").contains("orch/fix-it-today"));
    tui.ctrl('s');
    assert_eq!(create_requests(&mut tui).pop().unwrap().branch, None);
}

#[test]
fn ctrl_g_opens_the_editor_on_the_prompt_from_any_field() {
    let mut tui = form(config());
    tui.keys("draft");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.ctrl('g');
    assert!(tui.take_effects().contains(&Effect::EditText {
        text: "draft".into()
    }));
}

fn paste(tui: &mut Harness, text: &str) {
    tui.send(Event::Terminal(TermEvent::Paste(text.into())));
}

#[test]
fn a_paste_into_the_prompt_is_inserted_at_the_cursor_with_its_newlines() {
    let prompt = typed("before after", |tui| {
        with(tui, KeyCode::Left, KeyModifiers::CONTROL);
        paste(tui, "one\ntwo ");
    });
    assert_eq!(prompt, "before one\ntwo after");
}

#[test]
fn a_paste_into_branch_or_base_keeps_the_first_line_trimmed() {
    let mut tui = form(config());
    tui.keys("Fix");
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    tui.ctrl('u');
    paste(&mut tui, "  feature/x  \nmore");
    tui.press(KeyCode::Tab);
    paste(&mut tui, "\trelease/2\n");
    tui.ctrl('s');
    let create = create_requests(&mut tui).pop().unwrap();
    assert_eq!(create.branch.as_deref(), Some("feature/x"));
    assert_eq!(create.base.as_deref(), Some("release/2"));
}

#[test]
fn a_paste_on_the_repo_field_opens_the_picker_with_it_as_the_filter() {
    let mut tui = form(config());
    tui.press(KeyCode::Tab);
    paste(&mut tui, "old");
    assert!(tui.line_with("> old").contains("> old"));
    assert!(tui.line_with("▸").contains("/home/me/older"));
}

#[test]
fn a_paste_on_the_preset_is_ignored() {
    let mut tui = form(config());
    tui.keys("Fix");
    shift_tab(&mut tui);
    paste(&mut tui, "plan");
    tui.ctrl('s');
    let create = create_requests(&mut tui).pop().unwrap();
    assert_eq!(create.preset, None);
    assert_eq!(create.prompt, "Fix");
}

#[test]
fn a_paste_into_the_picker_filter_is_inserted() {
    let mut tui = form(config());
    tui.ctrl('r');
    paste(&mut tui, "me/old");
    assert!(tui.line_with("▸").contains("/home/me/older"));
    assert!(create_requests(&mut tui).is_empty());
}

fn row_of(tui: &mut Harness, needle: &str) -> usize {
    let lines = tui.lines();
    lines
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} not in\n{}", lines.join("\n")))
}

fn column_of(tui: &mut Harness, needle: &str) -> u16 {
    let line = tui.line_with(needle);
    line[..line.find(needle).unwrap()].chars().count() as u16
}

#[test]
fn fields_are_stacked_left_of_the_prompt_with_labels_above_values() {
    let mut tui = form(config());
    let repo = row_of(&mut tui, "│ Repo ");
    assert!(tui.lines()[repo].contains("Prompt"));
    assert!(tui.lines()[repo + 1].contains("/home/me/recent"));
    assert!(row_of(&mut tui, "│ Branch ") > repo + 1);
    assert!(row_of(&mut tui, "│ Preset ") > row_of(&mut tui, "│ Base "));
    assert!(column_of(&mut tui, "Prompt") > column_of(&mut tui, "/home/me/recent"));
}

#[test]
fn below_about_100_columns_the_fields_are_one_line_rows_above_a_full_width_prompt() {
    let mut tui = form(config());
    tui.resize(90, 30);
    assert!(tui.line_with("│ Repo ").contains("/home/me/recent"));
    assert!(tui.line_with("│ Branch ").contains("orch/"));
    assert!(row_of(&mut tui, "Prompt") > row_of(&mut tui, "│ Preset "));
}

#[test]
fn a_long_prompt_is_word_wrapped_in_its_column() {
    let mut tui = form(config());
    let words: Vec<String> = (0..30).map(|at| format!("word{at:02}")).collect();
    tui.keys(&words.join(" "));
    for word in &words {
        assert!(tui.screen().contains(word.as_str()), "{word} is cut");
    }
    assert!(row_of(&mut tui, "word29") > row_of(&mut tui, "word00"));
}

#[test]
fn the_popup_keeps_its_size_while_typing() {
    let mut tui = form(config());
    let top = row_of(&mut tui, ":new Session");
    let bottom = row_of(&mut tui, "Ctrl+s create");
    let lines: Vec<String> = (0..40).map(|at| format!("line {at}")).collect();
    tui.keys(&lines.join("\n"));
    assert_eq!(row_of(&mut tui, ":new Session"), top);
    assert_eq!(row_of(&mut tui, "Ctrl+s create"), bottom);
}

#[test]
fn an_overflowing_prompt_scrolls_to_keep_the_cursor_visible() {
    let mut tui = form(config());
    let lines: Vec<String> = (0..40).map(|at| format!("line {at:02}")).collect();
    tui.keys(&lines.join("\n"));
    let screen = tui.screen();
    assert!(screen.contains("line 39"), "{screen}");
    assert!(!screen.contains("line 00"), "{screen}");
    assert!(screen.contains("more lines"), "{screen}");

    for _ in 0..40 {
        tui.press(KeyCode::Up);
    }
    let screen = tui.screen();
    assert!(screen.contains("line 00"), "{screen}");
    assert!(!screen.contains("more lines"), "{screen}");
}

#[test]
fn the_terminal_cursor_sits_at_the_text_cursor() {
    let mut tui = form(config());
    tui.keys("abcd");
    tui.press(KeyCode::Left);
    let at = (
        column_of(&mut tui, "abcd") + 3,
        row_of(&mut tui, "abcd") as u16,
    );
    assert_eq!(tui.cursor(), Some(at));

    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    let branch = row_of(&mut tui, "orch/abcd") as u16;
    let end = column_of(&mut tui, "orch/abcd") + "orch/abcd".len() as u16;
    assert_eq!(tui.cursor(), Some((end, branch)));

    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Tab);
    assert_eq!(tui.cursor(), None);
}

fn hints(tui: &mut Harness) -> String {
    let lines = tui.lines();
    let at = row_of(tui, ":new Session");
    lines[at..]
        .iter()
        .find(|line| line.contains('╰'))
        .cloned()
        .unwrap()
}

#[test]
fn the_bottom_border_shows_the_keys_for_the_focused_field() {
    let mut tui = form(config());
    let prompt = hints(&mut tui);
    assert!(prompt.contains("Enter newline"), "{prompt}");
    assert!(prompt.contains("Ctrl+s create"), "{prompt}");

    tui.press(KeyCode::Tab);
    let repo = hints(&mut tui);
    assert!(repo.contains("Enter pick"), "{repo}");

    tui.press(KeyCode::Enter);
    let picker = hints(&mut tui);
    assert!(picker.contains("Esc back"), "{picker}");
    assert!(picker.contains("Tab descend"), "{picker}");
    tui.press(KeyCode::Esc);

    shift_tab(&mut tui);
    shift_tab(&mut tui);
    let preset = hints(&mut tui);
    assert!(preset.contains("←/→ choose"), "{preset}");
}

#[test]
fn the_focused_label_is_highlighted() {
    let mut tui = form(config());
    assert_eq!(tui.background_of("Prompt"), Color::Cyan);
    assert_ne!(tui.background_of("Repo "), Color::Cyan);
    tui.press(KeyCode::Tab);
    assert_eq!(tui.background_of("Repo "), Color::Cyan);
}

fn to_base(tui: &mut Harness) {
    for _ in 0..3 {
        tui.press(KeyCode::Tab);
    }
}

#[test]
fn up_and_down_on_base_cycle_other_sessions_branches_and_the_repo_default() {
    let mut other = session("recent", "other-work");
    other.branch = "orch/other-work".into();
    let mut tui = Harness::with_config(config());
    tui.sessions(vec![other]);
    tui.command("new");
    to_base(&mut tui);

    tui.press(KeyCode::Down);
    assert!(field(&mut tui, "Base").contains("orch/other-work"));
    tui.press(KeyCode::Down);
    assert!(field(&mut tui, "Base").contains("(Repo default)"));
    tui.press(KeyCode::Up);
    assert!(field(&mut tui, "Base").contains("orch/other-work"));

    tui.keys("release");
    assert!(field(&mut tui, "Base").contains("release"));
    assert!(!field(&mut tui, "Base").contains("orch/other-work"));
}

#[test]
fn ctrl_r_opens_the_repo_picker_and_esc_only_closes_it() {
    let mut tui = form(config());
    tui.keys("Keep me");
    tui.ctrl('r');
    let picker = tui.line_with("▸");
    assert!(picker.contains("recent"), "{picker}");
    assert!(picker.contains("/home/me/recent"), "{picker}");
    assert!(tui.screen().contains("/home/me/older"));

    tui.press(KeyCode::Esc);
    assert!(tui.screen().contains(":new Session"));
    assert!(!tui.screen().contains("/home/me/older"));
    assert_eq!(prompt_of_create(&mut tui), "Keep me");
}

#[test]
fn enter_in_the_picker_picks_the_highlighted_repo() {
    let mut tui = form(config());
    tui.keys("Work");
    tui.ctrl('r');
    tui.press(KeyCode::Down);
    tui.press(KeyCode::Enter);
    assert!(field(&mut tui, "Repo").contains("/home/me/older"));
    tui.keys(" more");
    tui.ctrl('s');
    let create = create_requests(&mut tui).pop().unwrap();
    assert_eq!(create.repo, std::path::PathBuf::from("/home/me/older"));
    assert_eq!(create.prompt, "Work more");
}

#[test]
fn ctrl_s_in_the_picker_creates_with_the_repo_already_chosen() {
    let mut tui = form(config());
    tui.keys("Work");
    tui.ctrl('r');
    tui.press(KeyCode::Down);
    tui.ctrl('s');
    let create = create_requests(&mut tui).pop().expect("no Session created");
    assert_eq!(create.repo, std::path::PathBuf::from("/home/me/recent"));
    assert_eq!(create.prompt, "Work");
}

#[test]
fn ctrl_s_in_the_picker_with_an_empty_prompt_shows_the_error_on_the_form() {
    let mut tui = form(config());
    tui.ctrl('r');
    tui.ctrl('s');
    assert!(create_requests(&mut tui).is_empty());
    assert!(
        tui.screen().contains("the prompt is empty"),
        "{}",
        tui.screen()
    );
    assert!(!tui.screen().contains("/home/me/older"), "{}", tui.screen());
}

#[test]
fn ctrl_g_in_the_picker_opens_the_editor_on_the_prompt() {
    let mut tui = form(config());
    tui.keys("draft");
    tui.ctrl('r');
    tui.ctrl('g');
    assert!(tui.take_effects().contains(&Effect::EditText {
        text: "draft".into()
    }));
}

#[test]
fn enter_or_typing_on_the_repo_field_opens_the_picker() {
    let mut tui = form(config());
    tui.press(KeyCode::Tab);
    tui.press(KeyCode::Enter);
    assert!(tui.screen().contains("/home/me/older"));
    tui.press(KeyCode::Esc);

    tui.keys("old");
    assert!(tui.line_with("> old").contains("> old"));
}

#[test]
fn up_and_down_on_preset_cycle_like_left_and_right() {
    let mut tui = form_with_presets(config());
    shift_tab(&mut tui);
    tui.press(KeyCode::Down);
    assert!(field(&mut tui, "Preset").contains("plan"));
    tui.press(KeyCode::Up);
    assert!(!field(&mut tui, "Preset").contains("plan"));
}
