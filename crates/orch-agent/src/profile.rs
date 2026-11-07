use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

const STAGED: &str = ".orch-new";
const RETIRED: &str = ".orch-old";

pub(crate) struct Dirs {
    pub(crate) home: PathBuf,
    pub(crate) user: PathBuf,
    pub(crate) profile: PathBuf,
}

impl Dirs {
    pub(crate) fn resolve(
        lookup: &dyn Fn(&str) -> Option<String>,
        agent: &str,
        user: impl FnOnce(&Path) -> Option<PathBuf>,
    ) -> Option<Self> {
        let set = |key: &str| lookup(key).filter(|value| !value.is_empty());
        let home = PathBuf::from(set("HOME")?);
        let state = set("XDG_STATE_HOME").map_or_else(|| home.join(".local/state"), PathBuf::from);
        Some(Self {
            user: user(&home)?,
            profile: state.join("orchestrator/agent-profiles").join(agent),
            home,
        })
    }
}

pub(crate) fn open_root(profile: &Path) -> io::Result<()> {
    std::fs::create_dir_all(profile)?;
    for dir in [Some(profile), profile.parent()].into_iter().flatten() {
        if !dir.symlink_metadata()?.is_dir() {
            return Err(io::Error::other(format!(
                "{} is not a directory",
                dir.display()
            )));
        }
    }
    std::fs::set_permissions(profile, std::fs::Permissions::from_mode(0o700))
}

pub(crate) fn create(path: &Path) -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

pub(crate) fn write_file(profile: &Path, name: &str, content: &[u8]) -> io::Result<()> {
    let staged = staged(profile, name)?;
    create(&staged)?.write_all(content)?;
    swap(profile, name, &staged)
}

pub(crate) fn link(profile: &Path, name: &str, target: &Path) -> io::Result<()> {
    if std::fs::read_link(profile.join(name)).is_ok_and(|current| current == target) {
        return Ok(());
    }
    let staged = staged(profile, name)?;
    std::os::unix::fs::symlink(target, &staged)?;
    swap(profile, name, &staged)
}

pub(crate) fn staged(profile: &Path, name: &str) -> io::Result<PathBuf> {
    let staged = profile.join(format!("{name}{STAGED}"));
    remove(&staged)?;
    Ok(staged)
}

pub(crate) fn swap(profile: &Path, name: &str, staged: &Path) -> io::Result<()> {
    let path = profile.join(name);
    match std::fs::rename(staged, &path) {
        Err(_) if path.symlink_metadata().is_ok() => {
            let retired = profile.join(format!("{name}{RETIRED}"));
            remove(&retired)?;
            std::fs::rename(&path, &retired)?;
            std::fs::rename(staged, &path)?;
            remove(&retired)
        }
        renamed => renamed,
    }
}

pub(crate) fn remove(path: &Path) -> io::Result<()> {
    let removed = match path.symlink_metadata() {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(err) => Err(err),
    };
    match removed {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

pub(crate) fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
