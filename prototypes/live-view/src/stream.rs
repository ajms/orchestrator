use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::DefaultTerminal;
use serde_json::{json, Value};

enum Msg {
    Out(String),
    Err(String),
    Closed,
}

struct Pending {
    request_id: String,
    tool: String,
    input: Value,
}

struct App {
    log: Vec<Line<'static>>,
    status: &'static str,
    since: Instant,
    pending: Option<Pending>,
    input: String,
    cost: f64,
    tokens_in: u64,
    tokens_out: u64,
    scroll_up: u16,
    next_id: u64,
}

impl App {
    fn set_status(&mut self, s: &'static str) {
        if s != self.status {
            self.status = s;
            self.since = Instant::now();
        }
    }
    fn push(&mut self, l: Line<'static>) {
        self.log.push(l);
    }
}

fn short(v: &Value, n: usize) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let one = s.replace('\n', " ⏎ ");
    if one.chars().count() > n {
        format!("{}…", one.chars().take(n).collect::<String>())
    } else {
        one
    }
}

fn handle(app: &mut App, line: &str) {
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => {
            app.push(Line::from(format!("?? {line}")).dark_gray());
            return;
        }
    };
    match v["type"].as_str().unwrap_or("") {
        "system" => {
            let sub = v["subtype"].as_str().unwrap_or("");
            if sub == "init" {
                app.push(
                    Line::from(format!(
                        "· init: model={} permissionMode={} tools={} slash_commands={}",
                        v["model"].as_str().unwrap_or("?"),
                        v["permissionMode"].as_str().unwrap_or("?"),
                        v["tools"].as_array().map(|a| a.len()).unwrap_or(0),
                        v["slash_commands"].as_array().map(|a| a.len()).unwrap_or(0),
                    ))
                    .dark_gray(),
                );
            } else {
                app.push(Line::from(format!("· system/{sub}")).dark_gray());
            }
        }
        "assistant" => {
            for c in v["message"]["content"].as_array().cloned().unwrap_or_default() {
                match c["type"].as_str().unwrap_or("") {
                    "text" => {
                        for l in c["text"].as_str().unwrap_or("").lines() {
                            app.push(Line::from(l.to_string()));
                        }
                    }
                    "tool_use" => app.push(Line::from(vec![
                        Span::styled("⚙ ", Style::new().fg(Color::Cyan)),
                        Span::styled(c["name"].as_str().unwrap_or("?").to_string(), Style::new().fg(Color::Cyan).bold()),
                        Span::raw(format!(" {}", short(&c["input"], 100))),
                    ])),
                    "thinking" => app.push(Line::from("(thinking…)").dark_gray().italic()),
                    other => app.push(Line::from(format!("· assistant/{other}")).dark_gray()),
                }
            }
            app.set_status("working");
        }
        "user" => {
            for c in v["message"]["content"].as_array().cloned().unwrap_or_default() {
                if c["type"] == "tool_result" {
                    let err = c["is_error"].as_bool().unwrap_or(false);
                    let content = match &c["content"] {
                        Value::Array(parts) => parts.iter().map(|p| p["text"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>().join(" "),
                        other => short(other, 400),
                    };
                    let l = Line::from(format!("  ↳ {}", short(&Value::String(content), 110)));
                    app.push(if err { l.red() } else { l.dark_gray() });
                }
            }
        }
        "result" => {
            app.cost = v["total_cost_usd"].as_f64().unwrap_or(app.cost);
            app.tokens_in += v["usage"]["input_tokens"].as_u64().unwrap_or(0)
                + v["usage"]["cache_read_input_tokens"].as_u64().unwrap_or(0)
                + v["usage"]["cache_creation_input_tokens"].as_u64().unwrap_or(0);
            app.tokens_out += v["usage"]["output_tokens"].as_u64().unwrap_or(0);
            app.push(
                Line::from(format!(
                    "── turn {} · {} turns · {} ms · ${:.4}",
                    v["subtype"].as_str().unwrap_or("?"),
                    v["num_turns"],
                    v["duration_ms"],
                    app.cost
                ))
                .magenta(),
            );
            app.set_status("idle");
        }
        "control_request" => {
            let req = &v["request"];
            if req["subtype"] == "can_use_tool" {
                let tool = req["tool_name"].as_str().unwrap_or("?").to_string();
                app.push(Line::from(format!("? permission requested: {tool}")).yellow());
                app.pending = Some(Pending {
                    request_id: v["request_id"].as_str().unwrap_or("").to_string(),
                    tool,
                    input: req["input"].clone(),
                });
                app.set_status("NEEDS INPUT");
            } else {
                app.push(Line::from(format!("· control_request {}", req["subtype"])).dark_gray());
            }
        }
        "control_response" => {
            let r = &v["response"];
            app.push(Line::from(format!("· control_response {} {}", r["subtype"], short(&r["response"], 80))).dark_gray());
        }
        "stream_event" => {}
        other => app.push(Line::from(format!("· {other}: {}", short(&v, 100))).dark_gray()),
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let [a] = Layout::horizontal([Constraint::Length(w)]).flex(Flex::Center).areas(area);
    let [a] = Layout::vertical([Constraint::Length(h)]).flex(Flex::Center).areas(a);
    a
}

pub fn run(terminal: &mut DefaultTerminal, dir: &Path) -> std::io::Result<()> {
    let session_id = uuid::Uuid::new_v4().to_string();
    let mut child = Command::new("claude")
        .args([
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--permission-mode",
            "manual",
            "--permission-prompt-tool",
            "stdio",
            "--session-id",
            &session_id,
        ])
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().unwrap();
    let (tx, rx) = mpsc::channel();
    {
        let tx = tx.clone();
        let out = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            for l in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx.send(Msg::Out(l));
            }
            let _ = tx.send(Msg::Closed);
        });
    }
    {
        let err = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            for l in BufReader::new(err).lines().map_while(Result::ok) {
                let _ = tx.send(Msg::Err(l));
            }
        });
    }

    let mut send = |v: Value| {
        let _ = writeln!(stdin, "{v}");
        let _ = stdin.flush();
    };
    send(json!({"type": "control_request", "request_id": "init_0", "request": {"subtype": "initialize", "hooks": null}}));

    let mut app = App {
        log: vec![Line::from(format!("scratch repo: {}", dir.display())).dark_gray()],
        status: "starting",
        since: Instant::now(),
        pending: None,
        input: String::new(),
        cost: 0.0,
        tokens_in: 0,
        tokens_out: 0,
        scroll_up: 0,
        next_id: 1,
    };

    loop {
        while let Ok(m) = rx.try_recv() {
            match m {
                Msg::Out(l) => handle(&mut app, &l),
                Msg::Err(l) => app.push(Line::from(format!("stderr: {l}")).red()),
                Msg::Closed => {
                    app.push(Line::from("[claude process closed stdout]").red());
                    app.set_status("exited");
                }
            }
        }

        terminal.draw(|f| {
            let [head, body, input] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(3)]).areas(f.area());
            let color = match app.status {
                "NEEDS INPUT" => Color::Yellow,
                "working" => Color::Green,
                "exited" => Color::Red,
                _ => Color::Gray,
            };
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(format!(" {} ", app.status), Style::new().fg(Color::Black).bg(color).bold()),
                    Span::raw(format!(
                        " {}s · ${:.4} · in {} / out {} tok · id {} · Esc interrupt · PgUp/PgDn scroll · F10 quit",
                        app.since.elapsed().as_secs(),
                        app.cost,
                        app.tokens_in,
                        app.tokens_out,
                        &session_id[..8]
                    )),
                ])),
                head,
            );

            let block = Block::bordered().title(" stream-json session (rendered by us) ");
            let inner = block.inner(body);
            let para = Paragraph::new(Text::from(app.log.clone())).wrap(Wrap { trim: false });
            let total = para.line_count(inner.width) as u16;
            let max = total.saturating_sub(inner.height);
            let scroll = max.saturating_sub(app.scroll_up.min(max));
            f.render_widget(para.block(block).scroll((scroll, 0)), body);

            let hint = if app.pending.is_some() { " y allow · n deny " } else { " prompt · Enter send " };
            f.render_widget(
                Paragraph::new(format!("{}▏", app.input)).block(Block::bordered().title(hint)),
                input,
            );

            if let Some(p) = &app.pending {
                let area = centered(f.area(), 80.min(f.area().width), 14.min(f.area().height));
                let pretty = serde_json::to_string_pretty(&p.input).unwrap_or_default();
                let mut lines = vec![Line::from(format!("Allow {}?", p.tool)).bold().yellow(), Line::from("")];
                lines.extend(pretty.lines().take(9).map(|l| Line::from(l.to_string())));
                lines.push(Line::from(""));
                lines.push(Line::from("[y] allow   [n] deny").bold());
                f.render_widget(Clear, area);
                f.render_widget(
                    Paragraph::new(lines).wrap(Wrap { trim: false }).block(Block::bordered().title(" permission ").yellow()),
                    area,
                );
            }
        })?;

        if event::poll(Duration::from_millis(30))? {
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => match k.code {
                    KeyCode::F(10) => break,
                    KeyCode::PageUp => app.scroll_up = app.scroll_up.saturating_add(10),
                    KeyCode::PageDown => app.scroll_up = app.scroll_up.saturating_sub(10),
                    KeyCode::Char(c @ ('y' | 'n')) if app.pending.is_some() => {
                        let p = app.pending.take().unwrap();
                        let decision = if c == 'y' {
                            json!({"behavior": "allow", "updatedInput": p.input})
                        } else {
                            json!({"behavior": "deny", "message": "User denied in prototype"})
                        };
                        send(json!({"type": "control_response", "response": {"subtype": "success", "request_id": p.request_id, "response": decision}}));
                        app.push(Line::from(format!("  → {} {}", if c == 'y' { "allowed" } else { "denied" }, p.tool)).yellow());
                        app.set_status("working");
                    }
                    KeyCode::Esc => {
                        send(json!({"type": "control_request", "request_id": format!("int_{}", app.next_id), "request": {"subtype": "interrupt"}}));
                        app.next_id += 1;
                        app.push(Line::from("[interrupt sent]").yellow());
                    }
                    KeyCode::Enter if app.pending.is_none() && !app.input.trim().is_empty() => {
                        let text = std::mem::take(&mut app.input);
                        app.push(Line::from(format!("> {text}")).bold().blue());
                        send(json!({"type": "user", "message": {"role": "user", "content": text}, "parent_tool_use_id": null, "session_id": session_id}));
                        app.scroll_up = 0;
                        app.set_status("working");
                    }
                    KeyCode::Backspace => {
                        app.input.pop();
                    }
                    KeyCode::Char(c) if app.pending.is_none() => app.input.push(c),
                    _ => {}
                },
                Event::Paste(s) if app.pending.is_none() => app.input.push_str(&s),
                _ => {}
            }
        }
    }
    let _ = child.kill();
    Ok(())
}
