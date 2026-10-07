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
    for edit in edits {
        match &edit.after {
            Some(contents) => write_atomically(&edit.path, contents)?,
            None => match std::fs::remove_file(&edit.path) {
                Err(err) if err.kind() != std::io::ErrorKind::NotFound => return Err(err),
                _ => {}
            },
        }
    }
    Ok(())
}

fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    let path = &std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".orch-new");
    std::fs::write(&temporary, contents)?;
    std::fs::rename(&temporary, path)
}

pub(crate) fn read(path: &Path) -> Result<Option<String>, HookupError> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(HookupError(format!(
            "cannot read {}: {err}",
            path.display()
        ))),
    }
}

pub(crate) fn edit(
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
