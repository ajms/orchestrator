use orch_holder::{Emulator, ScreenSnapshot, Size};

fn emulator(rows: u16, cols: u16) -> Emulator {
    Emulator::new(Size { rows, cols }, 1000)
}

fn history(emulator: &mut Emulator, lines: usize) {
    for i in 0..lines {
        emulator.process(format!("line {i}\r\n").as_bytes());
    }
}

fn scrolled_to_top(snapshot: &ScreenSnapshot) -> String {
    let mut restored = snapshot.restore(1000);
    restored.process(b"\x1b[?1049l");
    restored.screen_mut().set_scrollback(usize::MAX);
    restored.screen().contents()
}

#[test]
fn snapshot_restores_visible_screen_with_colours_and_cursor() {
    let mut emulator = emulator(5, 20);
    emulator.process(b"plain\r\n\x1b[31mred\x1b[m text\r\nprompt> ");

    let snapshot = emulator.snapshot();
    let restored = snapshot.restore(1000);

    assert_eq!(snapshot.size, Size { rows: 5, cols: 20 });
    assert_eq!(restored.screen().contents(), "plain\nred text\nprompt> ");
    assert_eq!(restored.screen().cursor_position(), (2, 8));
    assert_eq!(
        restored.screen().cell(1, 0).unwrap().fgcolor(),
        vt100::Color::Idx(1)
    );
}

#[test]
fn snapshot_restores_scrollback_in_order() {
    let mut emulator = emulator(3, 10);
    history(&mut emulator, 8);

    let snapshot = emulator.snapshot();
    let mut restored = snapshot.restore(1000);

    assert_eq!(snapshot.scrollback.len(), 6);
    assert_eq!(restored.screen().contents(), "line 6\nline 7");
    restored.screen_mut().set_scrollback(6);
    assert_eq!(restored.screen().contents(), "line 0\nline 1\nline 2");
    restored.screen_mut().set_scrollback(4);
    assert_eq!(restored.screen().contents(), "line 2\nline 3\nline 4");
}

#[test]
fn snapshot_leaves_the_live_emulator_unscrolled() {
    let mut emulator = emulator(2, 10);
    emulator.process(b"a\r\nb\r\nc\r\nd");

    emulator.snapshot();

    assert_eq!(emulator.snapshot().text(), "c\nd");
}

#[test]
fn snapshot_carries_input_modes() {
    let mut emulator = emulator(2, 10);
    emulator.process(b"\x1b[?1h\x1b[?2004h");

    let snapshot = emulator.snapshot();
    let restored = snapshot.restore(0);

    assert!(snapshot.input_modes().application_cursor);
    assert!(snapshot.input_modes().bracketed_paste);
    assert!(restored.screen().application_cursor());
    assert!(restored.screen().bracketed_paste());
}

#[test]
fn snapshot_text_reads_the_visible_screen() {
    let mut emulator = emulator(3, 10);
    emulator.process(b"hello\r\nworld");

    assert_eq!(emulator.snapshot().text(), "hello\nworld");
}

#[test]
fn alternate_screen_snapshot_shows_the_alternate_view_and_keeps_main_history() {
    let mut emulator = emulator(3, 20);
    history(&mut emulator, 8);
    emulator.process(b"\x1b[?1049h\x1b[Hfull screen app");

    let snapshot = emulator.snapshot();
    let mut restored = snapshot.restore(1000);

    assert!(snapshot.alternate_screen());
    assert!(restored.screen().alternate_screen());
    assert_eq!(restored.screen().contents(), "full screen app");
    assert_eq!(snapshot.text(), emulator.snapshot().text());

    restored.process(b"\x1b[?1049l");
    emulator.process(b"\x1b[?1049l");

    assert_eq!(restored.screen().contents(), "line 6\nline 7");
    assert_eq!(emulator.snapshot().text(), "line 6\nline 7");
    let back = emulator.snapshot();
    assert!(!back.alternate_screen());
    assert!(scrolled_to_top(&back).starts_with("line 0\nline 1"));
    restored.screen_mut().set_scrollback(usize::MAX);
    assert!(restored.screen().contents().starts_with("line 0\nline 1"));
}

#[test]
fn alternate_screen_entry_split_across_reads_is_recognised() {
    let mut emulator = emulator(3, 20);
    history(&mut emulator, 5);
    emulator.process(b"\x1b[?10");
    emulator.process(b"49hALT");

    let snapshot = emulator.snapshot();

    assert!(snapshot.alternate_screen());
    assert_eq!(snapshot.text(), "ALT");
    assert!(scrolled_to_top(&snapshot).starts_with("line 0\n"));
}

#[test]
fn legacy_alternate_screen_mode_47_keeps_main_history_too() {
    let mut emulator = emulator(3, 20);
    history(&mut emulator, 5);
    emulator.process(b"\x1b[?47hALT");

    let snapshot = emulator.snapshot();

    assert!(snapshot.alternate_screen());
    assert!(scrolled_to_top(&snapshot).starts_with("line 0\n"));
}

#[test]
fn resize_while_on_the_alternate_screen_resizes_the_main_screen_too() {
    let mut emulator = emulator(3, 20);
    history(&mut emulator, 5);
    emulator.process(b"\x1b[?1049hALT");

    emulator.resize(Size { rows: 4, cols: 30 });
    let snapshot = emulator.snapshot();
    let mut restored = snapshot.restore(1000);
    restored.process(b"\x1b[?1049l");

    assert_eq!(snapshot.size, Size { rows: 4, cols: 30 });
    assert_eq!(restored.screen().size(), (4, 30));
    assert!(restored.screen().contents().contains("line 4"));
}
