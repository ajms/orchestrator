use orch_holder::{Emulator, Size};

const MAX_COPY: usize = 1 << 20;

fn copies_after(bytes: &[u8]) -> Vec<String> {
    let mut emulator = Emulator::new(Size::DEFAULT, 0);
    emulator.process(bytes);
    emulator.take_copies()
}

fn osc52(text: &str) -> Vec<u8> {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    format!("\x1b]52;c;{encoded}\x07").into_bytes()
}

#[test]
fn an_osc_52_copy_is_captured_decoded() {
    assert_eq!(copies_after(&osc52("hello")), vec!["hello".to_string()]);
}

#[test]
fn an_empty_copy_is_ignored_so_it_cannot_wipe_the_clipboard() {
    assert!(copies_after(b"\x1b]52;c;\x07").is_empty());
}

#[test]
fn a_copy_up_to_the_size_limit_is_kept_and_a_larger_one_dropped() {
    let largest = "x".repeat(MAX_COPY);
    assert_eq!(copies_after(&osc52(&largest)), vec![largest]);

    let oversized = "x".repeat(MAX_COPY + 1);
    assert!(copies_after(&osc52(&oversized)).is_empty());
}
