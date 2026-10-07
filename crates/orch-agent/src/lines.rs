use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

#[derive(Debug, Default)]
pub(crate) struct FollowedLines {
    path: PathBuf,
    offset: u64,
}

pub(crate) struct NewLines {
    pub(crate) reset: bool,
    pub(crate) lines: Vec<Vec<u8>>,
}

impl FollowedLines {
    pub(crate) fn read(&mut self, path: &Path) -> Option<NewLines> {
        let mut file = File::open(path).ok()?;
        let len = file.metadata().ok()?.len();
        let reset = self.path != path || len < self.offset;
        if reset {
            *self = Self {
                path: path.to_path_buf(),
                offset: 0,
            };
        }
        file.seek(SeekFrom::Start(self.offset)).ok()?;
        let mut reader = BufReader::new(file);
        let mut lines = Vec::new();
        let mut line = Vec::new();
        while matches!(reader.read_until(b'\n', &mut line), Ok(read) if read > 0 && line.ends_with(b"\n"))
        {
            self.offset += line.len() as u64;
            lines.push(std::mem::take(&mut line));
        }
        Some(NewLines { reset, lines })
    }
}

pub(crate) fn first_str(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key)?.as_str())
        .map(String::from)
}
