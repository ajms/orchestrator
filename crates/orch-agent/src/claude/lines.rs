use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub(super) struct FollowedLines {
    path: PathBuf,
    offset: u64,
}

pub(super) struct NewLines {
    pub(super) reset: bool,
    pub(super) lines: Vec<Vec<u8>>,
}

impl FollowedLines {
    pub(super) fn read(&mut self, path: &Path) -> Option<NewLines> {
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
