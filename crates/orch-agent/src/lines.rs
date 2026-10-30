use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use nix::fcntl::OFlag;

const READ_BUDGET: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileId {
    dev: u64,
    ino: u64,
}

#[derive(Debug)]
struct Followed {
    path: PathBuf,
    file: FileId,
    offset: u64,
}

#[derive(Debug, Default)]
pub(crate) struct FollowedLines {
    followed: Option<Followed>,
    skipping: bool,
}

pub(crate) struct NewLines {
    pub(crate) reset: bool,
    pub(crate) lines: Vec<Vec<u8>>,
}

impl FollowedLines {
    pub(crate) fn read(&mut self, path: &Path) -> Option<NewLines> {
        self.read_opened(open(path, None)?, path)
    }

    pub(crate) fn read_within(&mut self, root: &Path, path: &Path) -> Option<NewLines> {
        self.read_opened(open(path, Some(root))?, path)
    }

    fn read_opened(&mut self, mut file: File, path: &Path) -> Option<NewLines> {
        let meta = file.metadata().ok()?;
        let id = FileId {
            dev: meta.dev(),
            ino: meta.ino(),
        };
        let offset = match &self.followed {
            Some(followed)
                if followed.path == path
                    && followed.file == id
                    && followed.offset <= meta.len() =>
            {
                Some(followed.offset)
            }
            _ => None,
        };
        let reset = offset.is_none();
        if reset {
            self.skipping = false;
        }
        let followed = self.followed.insert(Followed {
            path: path.to_path_buf(),
            file: id,
            offset: offset.unwrap_or(0),
        });
        let mut chunk = Vec::new();
        file.seek(SeekFrom::Start(followed.offset)).ok()?;
        (&mut file).take(READ_BUDGET).read_to_end(&mut chunk).ok()?;
        let complete = chunk
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |at| at + 1);
        let mut lines = Vec::new();
        for line in chunk[..complete].split_inclusive(|byte| *byte == b'\n') {
            if !std::mem::take(&mut self.skipping) {
                lines.push(line.to_vec());
            }
        }
        followed.offset += complete as u64;
        if complete == 0 && chunk.len() as u64 == READ_BUDGET {
            followed.offset += READ_BUDGET;
            self.skipping = true;
        }
        Some(NewLines { reset, lines })
    }
}

fn open(path: &Path, root: Option<&Path>) -> Option<File> {
    let file = File::options()
        .read(true)
        .custom_flags((OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK).bits())
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.nlink() != 1 {
        return None;
    }
    if let Some(root) = root {
        let opened = std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).ok()?;
        if !opened.starts_with(root) {
            return None;
        }
    }
    Some(file)
}
