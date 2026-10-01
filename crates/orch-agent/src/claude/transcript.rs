use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use orch_core::AgentEvent;
use serde::Deserialize;

use super::hooks::HookEvent;
use crate::TitleWatch;

const CUSTOM_TITLE: &str = "custom-title";

#[derive(Debug, Default)]
pub(super) struct TranscriptTitles {
    transcript: Option<PathBuf>,
    reader: TitleReader,
    from_hook: Option<String>,
    reported: Option<String>,
}

#[derive(Deserialize)]
struct Followed {
    transcript_path: Option<String>,
    hook_event_name: Option<HookEvent>,
    session_title: Option<String>,
}

impl TitleWatch for TranscriptTitles {
    fn follow(&mut self, payload: &str) {
        let Ok(followed) = serde_json::from_str::<Followed>(payload) else {
            return;
        };
        if let Some(path) = followed.transcript_path.map(PathBuf::from)
            && self.transcript.as_ref() != Some(&path)
        {
            self.transcript = Some(path);
            self.from_hook = None;
        }
        if matches!(
            followed.hook_event_name,
            Some(HookEvent::SessionStart | HookEvent::UserPromptSubmit)
        ) && let Some(title) = followed.session_title
        {
            self.from_hook = Some(title);
        }
    }

    fn poll(&mut self) -> Vec<AgentEvent> {
        let from_transcript = match &self.transcript {
            Some(path) => self.reader.latest(path),
            None => None,
        };
        let title = from_transcript.or_else(|| self.from_hook.clone());
        if title.is_none() || title == self.reported {
            return Vec::new();
        }
        self.reported.clone_from(&title);
        title
            .map(|title| AgentEvent::TitleChanged { title })
            .into_iter()
            .collect()
    }
}

#[derive(Debug, Default)]
struct TitleReader {
    path: PathBuf,
    offset: u64,
    title: Option<String>,
}

#[derive(Deserialize)]
struct TranscriptLine {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "customTitle")]
    custom_title: Option<String>,
}

impl TitleReader {
    fn latest(&mut self, path: &Path) -> Option<String> {
        let mut file = File::open(path).ok()?;
        let len = file.metadata().ok()?.len();
        if self.path != path || len < self.offset {
            *self = Self {
                path: path.to_path_buf(),
                ..Self::default()
            };
        }
        file.seek(SeekFrom::Start(self.offset)).ok()?;
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        while matches!(reader.read_until(b'\n', &mut line), Ok(read) if read > 0 && line.ends_with(b"\n"))
        {
            self.offset += line.len() as u64;
            if let Some(title) = custom_title(&line) {
                self.title = Some(title);
            }
            line.clear();
        }
        self.title.clone()
    }
}

fn custom_title(line: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(line).ok()?;
    if !text.contains(CUSTOM_TITLE) {
        return None;
    }
    let entry: TranscriptLine = serde_json::from_str(text).ok()?;
    (entry.kind == CUSTOM_TITLE)
        .then_some(entry.custom_title)
        .flatten()
}
