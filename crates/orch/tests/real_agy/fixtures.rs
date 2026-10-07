use std::path::{Path, PathBuf};

use serde_json::Value;

pub const ROOT_ID: &str = "3c1e9a40-7d52-4b8e-a6f1-2d9b0c4e7a13";
pub const CHILD_ID: &str = "8d2f61b7-4a09-4c3e-9b15-e07a3c5d9f28";
pub const STATUSLINE: &str = "statusline";

#[derive(Debug, Clone, PartialEq)]
pub struct Capture {
    pub event: String,
    pub at: u128,
    pub payload: String,
}

pub struct Scrubbing {
    replacements: Vec<(String, String)>,
}

impl Scrubbing {
    pub fn new(mut replacements: Vec<(String, String)>) -> Self {
        replacements.retain(|(from, _)| !from.is_empty());
        replacements.sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
        Self { replacements }
    }

    pub fn scrub(&self, text: &str) -> String {
        let replaced = self
            .replacements
            .iter()
            .fold(text.to_string(), |text, (from, to)| text.replace(from, to));
        scrub_emails(&replaced)
    }
}

fn scrub_emails(text: &str) -> String {
    let local = |c: char| c.is_ascii_alphanumeric() || "._%+-".contains(c);
    let domain = |c: char| c.is_ascii_alphanumeric() || ".-".contains(c);
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('@') {
        let start = rest[..at]
            .char_indices()
            .rev()
            .take_while(|(_, c)| local(*c))
            .last()
            .map_or(at, |(i, _)| i);
        let end = at
            + 1
            + rest[at + 1..]
                .find(|c| !domain(c))
                .unwrap_or(rest.len() - at - 1);
        let host = rest[at + 1..end].trim_end_matches('.');
        if start < at && host.contains('.') {
            out.push_str(&rest[..start]);
            out.push_str("dev@example.com");
            rest = &rest[at + 1 + host.len()..];
        } else {
            out.push_str(&rest[..=at]);
            rest = &rest[at + 1..];
        }
    }
    out.push_str(rest);
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Source {
    Root,
    Child,
    Line,
}

struct Rule {
    fixture: &'static str,
    event: &'static str,
    source: Source,
    matches: fn(&Value) -> bool,
}

fn tool(payload: &Value, name: &str) -> bool {
    payload["toolCall"]["name"] == name
}

const RULES: [Rule; 16] = [
    Rule {
        fixture: "hooks/post_invocation.json",
        event: "PostInvocation",
        source: Source::Root,
        matches: |_| true,
    },
    Rule {
        fixture: "hooks/post_tool_use.json",
        event: "PostToolUse",
        source: Source::Root,
        matches: |_| true,
    },
    Rule {
        fixture: "hooks/pre_invocation.json",
        event: "PreInvocation",
        source: Source::Root,
        matches: |_| true,
    },
    Rule {
        fixture: "hooks/pre_tool_use_invoke_subagent.json",
        event: "PreToolUse",
        source: Source::Root,
        matches: |p| tool(p, "invoke_subagent"),
    },
    Rule {
        fixture: "hooks/pre_tool_use_run_command.json",
        event: "PreToolUse",
        source: Source::Root,
        matches: |p| tool(p, "run_command"),
    },
    Rule {
        fixture: "hooks/pre_tool_use_write_to_file.json",
        event: "PreToolUse",
        source: Source::Root,
        matches: |p| tool(p, "write_to_file"),
    },
    Rule {
        fixture: "hooks/stop_fully_idle.json",
        event: "Stop",
        source: Source::Root,
        matches: |p| p["fullyIdle"] == true,
    },
    Rule {
        fixture: "hooks/stop_waiting_on_subagent.json",
        event: "Stop",
        source: Source::Root,
        matches: |p| p["fullyIdle"] == false,
    },
    Rule {
        fixture: "hooks/subagent_post_tool_use.json",
        event: "PostToolUse",
        source: Source::Child,
        matches: |_| true,
    },
    Rule {
        fixture: "hooks/subagent_pre_invocation.json",
        event: "PreInvocation",
        source: Source::Child,
        matches: |_| true,
    },
    Rule {
        fixture: "hooks/subagent_pre_tool_use_run_command.json",
        event: "PreToolUse",
        source: Source::Child,
        matches: |p| tool(p, "run_command"),
    },
    Rule {
        fixture: "hooks/subagent_stop.json",
        event: "Stop",
        source: Source::Child,
        matches: |_| true,
    },
    Rule {
        fixture: "statusline/idle.json",
        event: STATUSLINE,
        source: Source::Line,
        matches: |p| {
            p["agent_state"] == "idle"
                && p["conversation_id"]
                    .as_str()
                    .is_some_and(|id| !id.is_empty())
        },
    },
    Rule {
        fixture: "statusline/tool_confirmation_accept_edits.json",
        event: STATUSLINE,
        source: Source::Line,
        matches: |p| p["tool_confirmation_pending"] == true && p["cycle_mode"] == "accept-edits",
    },
    Rule {
        fixture: "statusline/trust_screen.json",
        event: STATUSLINE,
        source: Source::Line,
        matches: |p| p["tool_confirmation_pending"] == true && p["agent_state"] == "initializing",
    },
    Rule {
        fixture: "statusline/working_plan.json",
        event: STATUSLINE,
        source: Source::Line,
        matches: |p| p["agent_state"] == "working" && p["cycle_mode"] == "plan",
    },
];

fn source(capture: &Capture, payload: &Value, root: &str) -> Source {
    match (capture.event.as_str(), payload["conversationId"].as_str()) {
        (STATUSLINE, _) => Source::Line,
        (_, Some(id)) if id == root => Source::Root,
        _ => Source::Child,
    }
}

pub fn pick<'a>(captures: &'a [Capture], root: &str) -> Vec<(&'static str, &'a Capture)> {
    let parsed: Vec<(&Capture, Value)> = captures
        .iter()
        .filter_map(|capture| Some((capture, serde_json::from_str(&capture.payload).ok()?)))
        .collect();
    RULES
        .iter()
        .filter_map(|rule| {
            parsed
                .iter()
                .filter(|(capture, payload)| {
                    capture.event == rule.event
                        && source(capture, payload, root) == rule.source
                        && (rule.matches)(payload)
                })
                .min_by_key(|(capture, _)| capture.at)
                .map(|(capture, _)| (rule.fixture, *capture))
        })
        .collect()
}

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../orch-agent/tests/fixtures/antigravity")
}

pub fn read_captures(dir: &Path) -> Vec<Capture> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let stem = path.file_stem()?.to_str()?;
            let (event, at) = stem.rsplit_once('-')?;
            Some(Capture {
                event: event.into(),
                at: at.parse().ok()?,
                payload: std::fs::read_to_string(&path).ok()?,
            })
        })
        .collect()
}

fn child_conversation(captures: &[Capture], root: &str) -> Option<(String, Option<String>)> {
    let mut hooks: Vec<&Capture> = captures
        .iter()
        .filter(|capture| capture.event != STATUSLINE)
        .collect();
    hooks.sort_by_key(|capture| capture.at);
    hooks.into_iter().find_map(|capture| {
        let payload: Value = serde_json::from_str(&capture.payload).ok()?;
        let id = payload["conversationId"]
            .as_str()
            .filter(|id| *id != root)?;
        let transcript = payload["transcriptPath"].as_str().map(String::from);
        Some((id.into(), transcript))
    })
}

pub fn record(captures: &Path, root: &str, mut replacements: Vec<(String, String)>) -> Vec<String> {
    let captures = read_captures(captures);
    let child = child_conversation(&captures, root);
    replacements.push((root.into(), ROOT_ID.into()));
    if let Some((id, _)) = &child {
        replacements.push((id.clone(), CHILD_ID.into()));
    }
    let scrubbing = Scrubbing::new(replacements);
    let out = fixtures_dir();
    let mut written = Vec::new();
    let mut write = |fixture: &str, text: &str| {
        let text = format!("{}\n", scrubbing.scrub(text.trim_end()));
        std::fs::write(out.join(fixture), text).unwrap();
        written.push(fixture.to_string());
    };
    for (fixture, capture) in pick(&captures, root) {
        write(fixture, &capture.payload);
    }
    if let Some(transcript) = child
        .and_then(|(_, path)| path)
        .and_then(|path| std::fs::read_to_string(path).ok())
    {
        write("transcripts/subagent_full.jsonl", &transcript);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture(event: &str, at: u128, payload: &str) -> Capture {
        Capture {
            event: event.into(),
            at,
            payload: payload.into(),
        }
    }

    #[test]
    fn scrubbing_replaces_paths_longest_first_and_every_email() {
        let scrubbing = Scrubbing::new(vec![
            ("/tmp/od1/home".into(), "/home/dev".into()),
            (
                "/tmp/od1/repos/app/.orchestrator/worktrees/fix".into(),
                "/home/dev/shop/.orchestrator/worktrees/fix-login".into(),
            ),
            ("/tmp/od1/repos/app".into(), "/home/dev/shop".into()),
            ("real-conversation".into(), ROOT_ID.into()),
        ]);
        let raw = r#"{"conversationId":"real-conversation","cwd":"/tmp/od1/repos/app/.orchestrator/worktrees/fix","repo":"/tmp/od1/repos/app","transcriptPath":"/tmp/od1/home/.gemini/x","email":"jane.doe+agy@mail.example.org","note":"ask ops@corp.io now"}"#;

        assert_eq!(
            scrubbing.scrub(raw),
            format!(
                r#"{{"conversationId":"{ROOT_ID}","cwd":"/home/dev/shop/.orchestrator/worktrees/fix-login","repo":"/home/dev/shop","transcriptPath":"/home/dev/.gemini/x","email":"dev@example.com","note":"ask dev@example.com now"}}"#
            )
        );
    }

    #[test]
    fn each_fixture_takes_the_first_capture_of_the_right_conversation() {
        let root = |fields: &str| format!(r#"{{"conversationId":"root",{fields}}}"#);
        let child = |fields: &str| format!(r#"{{"conversationId":"child",{fields}}}"#);
        let run_command = r#""toolCall":{"name":"run_command","args":{}}"#;
        let captures = [
            capture("PreToolUse", 1, &child(run_command)),
            capture("PreToolUse", 2, &root(run_command)),
            capture("PreToolUse", 3, &root(run_command)),
            capture("Stop", 4, &root(r#""fullyIdle":false"#)),
            capture("Stop", 5, &child(r#""fullyIdle":true"#)),
            capture("Stop", 6, &root(r#""fullyIdle":true"#)),
            capture(
                STATUSLINE,
                7,
                r#"{"conversation_id":"","agent_state":"initializing","tool_confirmation_pending":true}"#,
            ),
            capture(
                STATUSLINE,
                8,
                r#"{"conversation_id":"root","agent_state":"idle"}"#,
            ),
        ];

        let picked: Vec<(&str, u128)> = pick(&captures, "root")
            .into_iter()
            .map(|(fixture, capture)| (fixture, capture.at))
            .collect();

        assert_eq!(
            picked,
            [
                ("hooks/pre_tool_use_run_command.json", 2),
                ("hooks/stop_fully_idle.json", 6),
                ("hooks/stop_waiting_on_subagent.json", 4),
                ("hooks/subagent_pre_tool_use_run_command.json", 1),
                ("hooks/subagent_stop.json", 5),
                ("statusline/idle.json", 8),
                ("statusline/trust_screen.json", 7),
            ]
        );
    }
}
