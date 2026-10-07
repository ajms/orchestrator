use std::path::{Path, PathBuf};

use serde_json::Value;

pub const ROOT_ID: &str = "3c1e9a40-7d52-4b8e-a6f1-2d9b0c4e7a13";
pub const CHILD_ID: &str = "8d2f61b7-4a09-4c3e-9b15-e07a3c5d9f28";
pub const STATUSLINE: &str = "statusline";
const EMAIL: &str = "dev@example.com";
const SUBAGENT_TRANSCRIPT: &str = "transcripts/subagent_full.jsonl";

#[derive(Debug, Clone, PartialEq)]
pub struct Capture {
    pub event: String,
    pub at: u128,
    pub payload: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recording {
    Trust,
    Hookup,
    Permission,
    Subagent,
}

pub struct Conversations {
    pub root: String,
    pub subagents: Vec<String>,
}

pub struct Scrubbing {
    replacements: Vec<(String, String)>,
    forbidden: Vec<String>,
}

impl Scrubbing {
    pub fn new(mut replacements: Vec<(String, String)>, mut forbidden: Vec<String>) -> Self {
        replacements.retain(|(from, _)| !from.is_empty());
        replacements.sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
        forbidden.retain(|needle| !needle.is_empty());
        Self {
            replacements,
            forbidden,
        }
    }

    pub fn scrub(&self, text: &str) -> String {
        let text = self
            .replacements
            .iter()
            .fold(text.to_string(), |text, (from, to)| text.replace(from, to));
        let mut out = String::new();
        let mut copied = 0;
        for email in emails(&text) {
            out.push_str(&text[copied..email.start]);
            out.push_str(&match text[email.clone()].contains('@') {
                true => EMAIL.into(),
                false => EMAIL.replace('@', "%40"),
            });
            copied = email.end;
        }
        out.push_str(&text[copied..]);
        out
    }

    pub fn leaks(&self, text: &str) -> Vec<String> {
        let allowed = [EMAIL.to_string(), EMAIL.replace('@', "%40")];
        let needles = self
            .forbidden
            .iter()
            .filter(|needle| text.contains(needle.as_str()))
            .cloned();
        let foreign = emails(text)
            .into_iter()
            .map(|email| text[email].to_string())
            .filter(|email| !allowed.contains(email));
        needles.chain(foreign).collect()
    }
}

fn emails(text: &str) -> Vec<std::ops::Range<usize>> {
    let local = |c: char| c.is_ascii_alphanumeric() || "._%+-".contains(c);
    let domain = |c: char| c.is_ascii_alphanumeric() || ".-".contains(c);
    let mut found = Vec::new();
    let mut at = 0;
    while let Some(next) = text[at..].find(['@', '%']).map(|offset| at + offset) {
        let separator = match &text[next..] {
            rest if rest.starts_with('@') => 1,
            rest if rest.starts_with("%40") => 3,
            _ => {
                at = next + 1;
                continue;
            }
        };
        let start = text[..next]
            .char_indices()
            .rev()
            .take_while(|(_, c)| local(*c))
            .last()
            .map_or(next, |(index, _)| index);
        let host_start = next + separator;
        let host_len = text[host_start..]
            .find(|c| !domain(c))
            .unwrap_or(text.len() - host_start);
        let host = text[host_start..host_start + host_len].trim_end_matches(['.', '-']);
        let last_label = host.rsplit('.').next().unwrap_or_default();
        if start < next && host.contains('.') && last_label.chars().any(|c| c.is_ascii_alphabetic())
        {
            found.push(start..host_start + host.len());
            at = host_start + host.len();
        } else {
            at = host_start;
        }
    }
    found
}

pub fn spawned_subagents(root_transcript: &str) -> Vec<String> {
    const KEY: &str = "conversationId";
    let id = |c: char| c.is_ascii_hexdigit() || c == '-';
    root_transcript
        .lines()
        .filter(|line| line.contains("Created the following subagents"))
        .flat_map(|line| {
            line.match_indices(KEY).filter_map(move |(index, _)| {
                let rest = &line[index + KEY.len()..];
                let rest = rest.trim_start_matches(|c: char| !c.is_ascii_hexdigit());
                let candidate = &rest[..rest.find(|c| !id(c)).unwrap_or(rest.len())];
                (candidate.len() == 36).then(|| candidate.to_string())
            })
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq)]
enum Source {
    Root,
    Subagent,
    Line,
}

struct Rule {
    fixture: &'static str,
    owner: Recording,
    event: &'static str,
    source: Source,
    matches: fn(&Value) -> bool,
}

fn tool(payload: &Value, name: &str) -> bool {
    payload["toolCall"]["name"] == name
}

const fn rule(
    fixture: &'static str,
    owner: Recording,
    event: &'static str,
    source: Source,
    matches: fn(&Value) -> bool,
) -> Rule {
    Rule {
        fixture,
        owner,
        event,
        source,
        matches,
    }
}

const RULES: [Rule; 14] = [
    rule(
        "hooks/post_invocation.json",
        Recording::Hookup,
        "PostInvocation",
        Source::Root,
        |_| true,
    ),
    rule(
        "hooks/post_tool_use.json",
        Recording::Permission,
        "PostToolUse",
        Source::Root,
        |_| true,
    ),
    rule(
        "hooks/pre_invocation.json",
        Recording::Hookup,
        "PreInvocation",
        Source::Root,
        |_| true,
    ),
    rule(
        "hooks/pre_tool_use_invoke_subagent.json",
        Recording::Subagent,
        "PreToolUse",
        Source::Root,
        |p| tool(p, "invoke_subagent"),
    ),
    rule(
        "hooks/pre_tool_use_run_command.json",
        Recording::Hookup,
        "PreToolUse",
        Source::Root,
        |p| tool(p, "run_command"),
    ),
    rule(
        "hooks/stop_fully_idle.json",
        Recording::Hookup,
        "Stop",
        Source::Root,
        |p| p["fullyIdle"] == true,
    ),
    rule(
        "hooks/stop_waiting_on_subagent.json",
        Recording::Subagent,
        "Stop",
        Source::Root,
        |p| p["fullyIdle"] == false,
    ),
    rule(
        "hooks/subagent_post_tool_use.json",
        Recording::Subagent,
        "PostToolUse",
        Source::Subagent,
        |_| true,
    ),
    rule(
        "hooks/subagent_pre_invocation.json",
        Recording::Subagent,
        "PreInvocation",
        Source::Subagent,
        |_| true,
    ),
    rule(
        "hooks/subagent_pre_tool_use_run_command.json",
        Recording::Subagent,
        "PreToolUse",
        Source::Subagent,
        |p| tool(p, "run_command"),
    ),
    rule(
        "hooks/subagent_stop.json",
        Recording::Subagent,
        "Stop",
        Source::Subagent,
        |_| true,
    ),
    rule(
        "statusline/idle.json",
        Recording::Trust,
        STATUSLINE,
        Source::Line,
        |p| {
            p["agent_state"] == "idle"
                && p["conversation_id"]
                    .as_str()
                    .is_some_and(|id| !id.is_empty())
        },
    ),
    rule(
        "statusline/tool_confirmation_accept_edits.json",
        Recording::Permission,
        STATUSLINE,
        Source::Line,
        |p| p["tool_confirmation_pending"] == true && p["cycle_mode"] == "accept-edits",
    ),
    rule(
        "statusline/trust_screen.json",
        Recording::Trust,
        STATUSLINE,
        Source::Line,
        |p| p["agent_state"] == "initializing",
    ),
];

fn source(capture: &Capture, payload: &Value, conversations: &Conversations) -> Option<Source> {
    if capture.event == STATUSLINE {
        return Some(Source::Line);
    }
    let id = payload["conversationId"].as_str()?;
    match id == conversations.root {
        true => Some(Source::Root),
        false => conversations
            .subagents
            .iter()
            .any(|subagent| subagent == id)
            .then_some(Source::Subagent),
    }
}

pub fn pick<'a>(
    captures: &'a [Capture],
    conversations: &Conversations,
    owner: Recording,
) -> Vec<(&'static str, &'a Capture)> {
    let parsed: Vec<(&Capture, Value)> = captures
        .iter()
        .filter_map(|capture| Some((capture, serde_json::from_str(&capture.payload).ok()?)))
        .collect();
    RULES
        .iter()
        .filter(|rule| rule.owner == owner)
        .filter_map(|rule| {
            parsed
                .iter()
                .filter(|(capture, payload)| {
                    capture.event == rule.event
                        && source(capture, payload, conversations) == Some(rule.source)
                        && (rule.matches)(payload)
                })
                .min_by_key(|(capture, _)| capture.at)
                .map(|(capture, _)| (rule.fixture, *capture))
        })
        .collect()
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

pub fn fully_idle_stop(captures: &[Capture], root: &str) -> bool {
    captures
        .iter()
        .filter(|capture| capture.event == "Stop")
        .any(|capture| {
            serde_json::from_str::<Value>(&capture.payload).is_ok_and(|payload| {
                payload["conversationId"] == root && payload["fullyIdle"] == true
            })
        })
}

fn transcript_of(captures: &[Capture], conversation: &str) -> Option<String> {
    captures.iter().find_map(|capture| {
        let payload: Value = serde_json::from_str(&capture.payload).ok()?;
        if payload["conversationId"] != conversation {
            return None;
        }
        payload["transcriptPath"].as_str().map(String::from)
    })
}

pub fn record(
    captures: &Path,
    conversations: &Conversations,
    owner: Recording,
    mut replacements: Vec<(String, String)>,
    forbidden: Vec<String>,
) -> Vec<String> {
    let captures = read_captures(captures);
    replacements.push((conversations.root.clone(), ROOT_ID.into()));
    if let Some(subagent) = conversations.subagents.first() {
        replacements.push((subagent.clone(), CHILD_ID.into()));
    }
    let scrubbing = Scrubbing::new(replacements, forbidden);
    let mut files: Vec<(String, String)> = pick(&captures, conversations, owner)
        .into_iter()
        .map(|(fixture, capture)| (fixture.into(), capture.payload.clone()))
        .collect();
    if owner == Recording::Subagent
        && let Some(transcript) = conversations
            .subagents
            .first()
            .and_then(|subagent| transcript_of(&captures, subagent))
            .and_then(|path| std::fs::read_to_string(path).ok())
    {
        files.push((SUBAGENT_TRANSCRIPT.into(), transcript));
    }
    let scrubbed: Vec<(String, String)> = files
        .into_iter()
        .map(|(fixture, text)| {
            let text = scrubbing.scrub(text.trim_end());
            let leaks = scrubbing.leaks(&text);
            assert!(leaks.is_empty(), "{fixture} still holds {leaks:?}:\n{text}");
            (fixture, text)
        })
        .collect();
    let out = fixtures_dir();
    for (fixture, text) in &scrubbed {
        std::fs::write(out.join(fixture), format!("{text}\n")).unwrap();
    }
    scrubbed.into_iter().map(|(fixture, _)| fixture).collect()
}

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../orch-agent/tests/fixtures/antigravity")
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

    fn scrubbing() -> Scrubbing {
        Scrubbing::new(
            vec![
                ("/tmp/od1/home".into(), "/home/dev".into()),
                (
                    "/tmp/od1/repos/app/.orchestrator/worktrees/fix".into(),
                    "/home/dev/shop/.orchestrator/worktrees/fix-login".into(),
                ),
                ("/tmp/od1/repos/app".into(), "/home/dev/shop".into()),
                ("real-conversation".into(), ROOT_ID.into()),
            ],
            vec!["/home/jane".into(), "jane".into(), "/tmp/".into()],
        )
    }

    #[test]
    fn scrubbing_replaces_paths_longest_first_and_every_email() {
        let raw = r#"{"conversationId":"real-conversation","cwd":"/tmp/od1/repos/app/.orchestrator/worktrees/fix","repo":"/tmp/od1/repos/app","transcriptPath":"/tmp/od1/home/.gemini/x","email":"jane.doe+agy@mail.example.org","note":"ask ops%40corp.io about pkg@1.2.3"}"#;

        let scrubbed = scrubbing().scrub(raw);

        assert_eq!(
            scrubbed,
            format!(
                r#"{{"conversationId":"{ROOT_ID}","cwd":"/home/dev/shop/.orchestrator/worktrees/fix-login","repo":"/home/dev/shop","transcriptPath":"/home/dev/.gemini/x","email":"dev@example.com","note":"ask dev%40example.com about pkg@1.2.3"}}"#
            )
        );
        assert_eq!(scrubbing().leaks(&scrubbed), Vec::<String>::new());
    }

    #[test]
    fn leaks_name_private_text_that_survived_scrubbing() {
        let text = r#"{"cwd":"/home/jane/x","log":"/tmp/agy.log","to":"bob@corp.io","cc":"amy%40corp.io","ok":"dev@example.com"}"#;

        assert_eq!(
            scrubbing().leaks(text),
            [
                "/home/jane",
                "jane",
                "/tmp/",
                "bob@corp.io",
                "amy%40corp.io"
            ]
        );
    }

    #[test]
    fn subagents_are_the_conversations_the_roots_invoke_subagent_created() {
        let result = r#"{"step_index": 4, "type": "GENERIC", "content": "Created the following subagents: {\"conversationId\": \"8d2f61b7-4a09-4c3e-9b15-e07a3c5d9f28\", \"logAbsoluteUri\": \"file:///x\"}"}"#;
        let message = r#"{"step_index": 5, "type": "SYSTEM_MESSAGE", "content": "[Message] sender=11111111-2222-3333-4444-555555555555 content=done"}"#;

        assert_eq!(
            spawned_subagents(&format!("{result}\n{message}\n")),
            [CHILD_ID]
        );
    }

    #[test]
    fn each_recording_writes_only_its_own_fixtures_from_known_conversations() {
        let root = |fields: &str| format!(r#"{{"conversationId":"root",{fields}}}"#);
        let child = |fields: &str| format!(r#"{{"conversationId":"child",{fields}}}"#);
        let stranger = |fields: &str| format!(r#"{{"conversationId":"stranger",{fields}}}"#);
        let run_command = r#""toolCall":{"name":"run_command","args":{}}"#;
        let captures = [
            capture("PreToolUse", 1, &stranger(run_command)),
            capture("PreToolUse", 2, &root(run_command)),
            capture("PreToolUse", 3, &root(run_command)),
            capture("Stop", 4, &root(r#""fullyIdle":false"#)),
            capture("Stop", 5, &stranger(r#""fullyIdle":true"#)),
            capture("Stop", 6, &child(r#""fullyIdle":true"#)),
            capture("Stop", 7, &root(r#""fullyIdle":true"#)),
            capture(
                STATUSLINE,
                8,
                r#"{"conversation_id":"","agent_state":"initializing"}"#,
            ),
        ];
        let conversations = Conversations {
            root: "root".into(),
            subagents: vec!["child".into()],
        };
        let picked = |owner| -> Vec<(&str, u128)> {
            pick(&captures, &conversations, owner)
                .into_iter()
                .map(|(fixture, capture)| (fixture, capture.at))
                .collect()
        };

        assert_eq!(
            picked(Recording::Hookup),
            [
                ("hooks/pre_tool_use_run_command.json", 2),
                ("hooks/stop_fully_idle.json", 7),
            ]
        );
        assert_eq!(
            picked(Recording::Subagent),
            [
                ("hooks/stop_waiting_on_subagent.json", 4),
                ("hooks/subagent_stop.json", 6),
            ]
        );
        assert_eq!(
            picked(Recording::Trust),
            [("statusline/trust_screen.json", 8)]
        );
    }
}
