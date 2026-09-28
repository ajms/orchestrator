use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph};
use ratatui::DefaultTerminal;
use serde_json::{json, Value};
use tui_term::widget::PseudoTerminal;

const HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionRequest",
    "PermissionDenied",
    "Notification",
    "Stop",
    "StopFailure",
    "SessionEnd",
];

struct Hooks {
    status: &'static str,
    since: Instant,
    log: Vec<(Instant, String)>,
}

fn status_for(ev: &Value, current: &'static str) -> &'static str {
    let name = ev["hook_event_name"].as_str().unwrap_or("");
    match name {
        "SessionStart" => "idle",
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "PostToolUseFailure" | "PermissionDenied" => "working",
        "PermissionRequest" => "NEEDS INPUT",
        "Notification" => match ev["notification_type"].as_str().unwrap_or("") {
            "permission_prompt" | "agent_needs_input" => "NEEDS INPUT",
            "idle_prompt" => "idle",
            _ => current,
        },
        "Stop" => "idle",
        "StopFailure" => "error",
        "SessionEnd" => "exited",
        _ => current,
    }
}

fn summary(ev: &Value) -> String {
    let name = ev["hook_event_name"].as_str().unwrap_or("?");
    let detail = ev["tool_name"]
        .as_str()
        .or(ev["notification_type"].as_str())
        .or(ev["source"].as_str())
        .unwrap_or("");
    format!("{name} {detail}")
}

fn key_bytes(k: KeyEvent, app_cursor: bool) -> Vec<u8> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    let shift = k.modifiers.contains(KeyModifiers::SHIFT);
    let arrow = |c: u8| if app_cursor { vec![0x1b, b'O', c] } else { vec![0x1b, b'[', c] };
    let mut out = match k.code {
        KeyCode::Char(c) if ctrl => {
            let c = c.to_ascii_lowercase();
            match c {
                'a'..='z' => vec![c as u8 - b'a' + 1],
                ' ' => vec![0],
                '[' => vec![0x1b],
                '\\' => vec![0x1c],
                ']' => vec![0x1d],
                _ => c.to_string().into_bytes(),
            }
        }
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter if shift || alt => return b"\x1b\r".to_vec(),
        KeyCode::Enter => b"\r".to_vec(),
        KeyCode::Tab => b"\t".to_vec(),
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => b"\x7f".to_vec(),
        KeyCode::Esc => b"\x1b".to_vec(),
        KeyCode::Up => arrow(b'A'),
        KeyCode::Down => arrow(b'B'),
        KeyCode::Right => arrow(b'C'),
        KeyCode::Left => arrow(b'D'),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::F(n @ 1..=4) => vec![0x1b, b'O', b'P' + n - 1],
        _ => vec![],
    };
    if alt && !out.is_empty() {
        out.insert(0, 0x1b);
    }
    out
}

pub fn run(terminal: &mut DefaultTerminal, dir: &Path) -> std::io::Result<()> {
    let events_path = dir.join(".git").join("orch-hook-events.jsonl");
    std::fs::write(&events_path, "")?;
    let p = events_path.display();
    let hook_cmd = format!("tr -d '\\n' >> '{p}'; echo >> '{p}'");
    let mut hooks_cfg = serde_json::Map::new();
    for e in HOOK_EVENTS {
        hooks_cfg.insert(
            e.to_string(),
            json!([{ "matcher": "*", "hooks": [{ "type": "command", "command": hook_cmd }] }]),
        );
    }
    let settings = json!({ "hooks": hooks_cfg }).to_string();
    let session_id = uuid::Uuid::new_v4().to_string();

    let (rows, cols) = (24u16, 80u16);
    let pty = native_pty_system()
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let mut cmd = CommandBuilder::new("claude");
    cmd.args(["--session-id", &session_id, "--settings", &settings]);
    cmd.cwd(dir);
    cmd.env("TERM", "xterm-256color");
    let mut child = pty.slave.spawn_command(cmd).map_err(|e| std::io::Error::other(e.to_string()))?;
    drop(pty.slave);
    let mut reader = pty.master.try_clone_reader().map_err(|e| std::io::Error::other(e.to_string()))?;
    let mut writer = pty.master.take_writer().map_err(|e| std::io::Error::other(e.to_string()))?;

    let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 5000)));
    let bytes_seen = Arc::new(Mutex::new(0usize));
    {
        let parser = parser.clone();
        let bytes_seen = bytes_seen.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                parser.lock().unwrap().process(&buf[..n]);
                *bytes_seen.lock().unwrap() += n;
            }
        });
    }

    let hooks = Arc::new(Mutex::new(Hooks { status: "starting", since: Instant::now(), log: vec![] }));
    {
        let hooks = hooks.clone();
        let events_path = events_path.clone();
        std::thread::spawn(move || {
            let mut offset = 0usize;
            loop {
                if let Ok(s) = std::fs::read_to_string(&events_path) {
                    if s.len() > offset {
                        for line in s[offset..].lines().filter(|l| !l.trim().is_empty()) {
                            let ev: Value = serde_json::from_str(line).unwrap_or(json!({"hook_event_name": "unparseable"}));
                            let mut h = hooks.lock().unwrap();
                            let next = status_for(&ev, h.status);
                            if next != h.status {
                                h.status = next;
                                h.since = Instant::now();
                            }
                            h.log.push((Instant::now(), summary(&ev)));
                        }
                        offset = s.len();
                    }
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
    }

    let start = Instant::now();
    let mut pane_size = (rows, cols);
    let mut scrollback = 0usize;
    let mut exited: Option<String> = None;

    loop {
        if exited.is_none() {
            if let Ok(Some(st)) = child.try_wait() {
                exited = Some(format!("{st:?}"));
            }
        }
        terminal.draw(|f| {
            let [left, right] = Layout::horizontal([Constraint::Length(36), Constraint::Min(20)]).areas(f.area());
            let [sessions, log] = Layout::vertical([Constraint::Length(7), Constraint::Min(3)]).areas(left);

            let h = hooks.lock().unwrap();
            let color = match h.status {
                "NEEDS INPUT" => Color::Yellow,
                "working" => Color::Green,
                "error" | "exited" => Color::Red,
                _ => Color::Gray,
            };
            let sess = vec![
                Line::from(vec![Span::raw("● "), Span::styled("proto-session", Style::new().bold())]),
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(h.status, Style::new().fg(color).bold()),
                    Span::raw(format!(" {}s", h.since.elapsed().as_secs())),
                ]),
                Line::from(format!("  pty bytes: {}", bytes_seen.lock().unwrap())).dark_gray(),
                Line::from(format!("  id: {}", &session_id[..8])).dark_gray(),
                Line::from(exited.as_deref().map(|e| format!("  child exited: {e}")).unwrap_or_default()).red(),
            ];
            f.render_widget(
                Paragraph::new(sess).block(Block::bordered().title(" Sessions (status = hooks only) ")),
                sessions,
            );
            let items: Vec<ListItem> = h
                .log
                .iter()
                .rev()
                .take(log.height as usize)
                .map(|(t, s)| ListItem::new(format!("{:>5.1}s {s}", (*t - start).as_secs_f32())))
                .collect();
            f.render_widget(List::new(items).block(Block::bordered().title(" hook events (newest first) ")), log);
            drop(h);

            let block = Block::bordered().title(format!(
                " claude (embedded PTY) · F10 quit · F7/F8 scroll{} ",
                if scrollback > 0 { format!(" [scrolled {scrollback}]") } else { String::new() }
            ));
            let inner = block.inner(right);
            if (inner.height, inner.width) != pane_size && inner.height > 0 && inner.width > 0 {
                pane_size = (inner.height, inner.width);
                let _ = pty.master.resize(PtySize { rows: inner.height, cols: inner.width, pixel_width: 0, pixel_height: 0 });
                parser.lock().unwrap().set_size(inner.height, inner.width);
            }
            let mut p = parser.lock().unwrap();
            p.set_scrollback(scrollback);
            f.render_widget(PseudoTerminal::new(p.screen()).block(block), right);
        })?;

        if event::poll(Duration::from_millis(30))? {
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => match k.code {
                    KeyCode::F(10) => break,
                    KeyCode::F(7) => scrollback += 5,
                    KeyCode::F(8) => scrollback = scrollback.saturating_sub(5),
                    _ => {
                        scrollback = 0;
                        let app_cursor = parser.lock().unwrap().screen().application_cursor();
                        let bytes = key_bytes(k, app_cursor);
                        if !bytes.is_empty() {
                            let _ = writer.write_all(&bytes);
                            let _ = writer.flush();
                        }
                    }
                },
                Event::Paste(s) => {
                    let bracketed = parser.lock().unwrap().screen().bracketed_paste();
                    let data = if bracketed { format!("\x1b[200~{s}\x1b[201~") } else { s };
                    let _ = writer.write_all(data.as_bytes());
                    let _ = writer.flush();
                }
                _ => {}
            }
        }
    }
    let _ = child.kill();
    Ok(())
}
