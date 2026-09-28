// PROTOTYPE — throwaway. Answers "embed native TUI or build our own?" (issue #7).
mod embed;
mod stream;

use std::path::PathBuf;
use std::process::Command;

const FIZZ: &str = r#"def fizzbuzz(n):
    for i in range(1, n):
        if i % 3 == 0 and i % 5 == 0:
            print("FizzBuzz")
        elif i % 3 == 0:
            print("Fizz")
        elif i % 3 == 0:
            print("Buzz")
        else:
            print(i)


fizzbuzz(15)
"#;

fn scratch_repo() -> PathBuf {
    let base = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
    let id = uuid::Uuid::new_v4().to_string();
    let dir = PathBuf::from(base).join(format!("orch-proto-{}", &id[..8]));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("fizz.py"), FIZZ).unwrap();
    for args in [&["init", "-q"][..], &["add", "."], &["commit", "-qm", "init"]] {
        Command::new("git").args(args).current_dir(&dir).status().unwrap();
    }
    dir
}

fn main() -> std::io::Result<()> {
    let mode = std::env::args().nth(1).unwrap_or_default();
    if mode != "embed" && mode != "stream" {
        eprintln!("usage: prototype-live-view <embed|stream>");
        std::process::exit(2);
    }
    let dir = scratch_repo();
    let mut terminal = ratatui::init();
    crossterm::execute!(std::io::stdout(), crossterm::event::EnableBracketedPaste)?;
    let enhanced = crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
    if enhanced {
        crossterm::execute!(
            std::io::stdout(),
            crossterm::event::PushKeyboardEnhancementFlags(
                crossterm::event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            )
        )?;
    }
    let result = if mode == "embed" {
        embed::run(&mut terminal, &dir)
    } else {
        stream::run(&mut terminal, &dir)
    };
    if enhanced {
        crossterm::execute!(std::io::stdout(), crossterm::event::PopKeyboardEnhancementFlags)?;
    }
    crossterm::execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste)?;
    ratatui::restore();
    println!("scratch repo: {}", dir.display());
    result
}
