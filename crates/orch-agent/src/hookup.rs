use std::path::{Path, PathBuf};

pub type Lookup<'a> = &'a dyn Fn(&str) -> Option<String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEdit {
    pub path: PathBuf,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookupState {
    Missing,
    Installed,
    Broken(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookupError(pub String);

impl std::fmt::Display for HookupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub trait AgentHookup {
    fn install(&self, orch_program: &str, lookup: Lookup) -> Result<Vec<FileEdit>, HookupError>;
    fn uninstall(&self, lookup: Lookup) -> Result<Vec<FileEdit>, HookupError>;
    fn state(&self, orch_program: &str, lookup: Lookup) -> HookupState;
}

pub fn apply(edits: &[FileEdit]) -> std::io::Result<()> {
    edits.iter().try_for_each(ensure_unchanged)?;
    for edit in edits {
        ensure_unchanged(edit)?;
        match &edit.after {
            Some(contents) => write_atomically(&edit.path, contents)?,
            None => remove(&edit.path)?,
        }
    }
    Ok(())
}

fn ensure_unchanged(edit: &FileEdit) -> std::io::Result<()> {
    let now = read_optional(&edit.path).map_err(|err| std::io::Error::other(err.0))?;
    match now == edit.before {
        true => Ok(()),
        false => Err(std::io::Error::other(format!(
            "{} changed since the diff was shown; run the command again",
            edit.path.display()
        ))),
    }
}

fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    let path = &std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut temporary = tempfile::NamedTempFile::new_in(dir)?;
    temporary.write_all(contents.as_bytes())?;
    if let Ok(metadata) = std::fs::metadata(path) {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())?;
    }
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|err| err.error)?;
    sync_dir(dir)
}

fn remove(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
        Ok(()) => sync_dir(path.parent().unwrap_or(Path::new("."))),
    }
}

fn sync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

pub(crate) fn read_optional(path: &Path) -> Result<Option<String>, HookupError> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(HookupError(format!(
            "cannot read {}: {err}",
            path.display()
        ))),
    }
}

pub(crate) fn file_edit(
    path: PathBuf,
    before: Option<String>,
    after: Option<String>,
) -> Option<FileEdit> {
    (before != after).then_some(FileEdit {
        path,
        before,
        after,
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn change(path: &Path, before: Option<&str>, after: Option<&str>) -> FileEdit {
        FileEdit {
            path: path.to_path_buf(),
            before: before.map(Into::into),
            after: after.map(Into::into),
        }
    }

    #[test]
    fn apply_refuses_a_file_that_changed_since_the_edit_was_planned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "rewritten by agy").unwrap();

        let applied = apply(&[change(&path, Some("seen"), Some("orch's"))]);

        let err = applied.unwrap_err().to_string();
        assert!(err.contains("changed"), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "rewritten by agy");
    }

    #[test]
    fn apply_refuses_a_file_that_appeared_since_the_edit_was_planned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks.json");
        std::fs::write(&path, "new").unwrap();

        let applied = apply(&[change(&path, None, Some("orch's"))]);

        assert!(applied.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
    }

    #[test]
    fn apply_checks_every_file_before_writing_any() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.json");
        let second = dir.path().join("second.json");
        std::fs::write(&second, "rewritten").unwrap();

        let applied = apply(&[
            change(&first, None, Some("orch's")),
            change(&second, Some("seen"), Some("orch's")),
        ]);

        assert!(applied.is_err());
        assert!(!first.exists());
    }

    #[test]
    fn apply_keeps_the_files_permissions_and_leaves_no_stray_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "seen").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        apply(&[change(&path, Some("seen"), Some("orch's"))]).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "orch's");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
