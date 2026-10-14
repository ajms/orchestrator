use std::path::{Component, Path, PathBuf};

use orch_agent::SandboxPaths;
use serde::Deserialize;

use crate::error::ConfigProblem;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub(crate) enum SandboxSetting {
    Switch(bool),
    Table(SandboxLayer),
}

impl SandboxSetting {
    pub(crate) fn switch(&self) -> Option<bool> {
        match self {
            SandboxSetting::Switch(on) => Some(*on),
            SandboxSetting::Table(_) => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SandboxLayer {
    #[serde(default)]
    readable: Vec<String>,
    #[serde(default)]
    writable: Vec<String>,
    #[serde(default)]
    hidden: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SandboxList {
    Readable,
    Writable,
    Hidden,
}

impl SandboxList {
    pub fn name(self) -> &'static str {
        match self {
            SandboxList::Readable => "readable",
            SandboxList::Writable => "writable",
            SandboxList::Hidden => "hidden",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SandboxEntry {
    pub(crate) list: SandboxList,
    pub(crate) written: String,
    pub(crate) path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SandboxContext {
    pub(crate) home: Option<PathBuf>,
    pub(crate) orch_dirs: Vec<PathBuf>,
    pub(crate) carve_outs: Vec<PathBuf>,
}

impl SandboxContext {
    pub(crate) fn entries(
        &self,
        setting: Option<&SandboxSetting>,
    ) -> Result<Vec<SandboxEntry>, ConfigProblem> {
        let Some(SandboxSetting::Table(layer)) = setting else {
            return Ok(Vec::new());
        };
        let lists = [
            (SandboxList::Readable, &layer.readable),
            (SandboxList::Writable, &layer.writable),
            (SandboxList::Hidden, &layer.hidden),
        ];
        let mut entries = Vec::new();
        for (list, written) in lists {
            for written in written {
                let problem = |why: &'static str| ConfigProblem::SandboxPath {
                    list: list.name(),
                    path: written.clone(),
                    why,
                };
                let path = self.expand(written).map_err(problem)?;
                if list != SandboxList::Hidden {
                    self.check_walls(&path).map_err(problem)?;
                    self.check_walls(&resolved(&path)).map_err(problem)?;
                }
                entries.push(SandboxEntry {
                    list,
                    written: written.clone(),
                    path,
                });
            }
        }
        Ok(entries)
    }

    fn expand(&self, written: &str) -> Result<PathBuf, &'static str> {
        let path = match (written.strip_prefix("~/"), &self.home) {
            (Some(rest), Some(home)) => home.join(rest),
            (Some(_), None) => return Err("needs HOME to be set"),
            (None, _) if Path::new(written).is_absolute() => PathBuf::from(written),
            (None, _) => return Err("must be absolute or start with ~/"),
        };
        Ok(normalized(&path))
    }

    fn check_walls(&self, path: &Path) -> Result<(), &'static str> {
        if path == Path::new("/") {
            return Err("is the root, a hard wall");
        }
        if walls(&self.home).any(|home| home.starts_with(path)) {
            return Err("is $HOME or contains it, a hard wall");
        }
        if walls(&self.orch_dirs).any(|dir| dir.starts_with(path) || path.starts_with(dir)) {
            return Err("overlaps orch's config or state, a hard wall");
        }
        if in_git_hooks_or_config(path) {
            return Err("is git's hooks or config, a hard wall");
        }
        if walls(&self.carve_outs).any(|carve_out| path.starts_with(carve_out)) {
            return Err("is Claude's read-only config, a hard wall");
        }
        Ok(())
    }
}

fn walls<'a>(paths: impl IntoIterator<Item = &'a PathBuf>) -> impl Iterator<Item = PathBuf> {
    paths
        .into_iter()
        .flat_map(|path| [normalized(path), resolved(path)])
}

fn resolved(path: &Path) -> PathBuf {
    let path = normalized(path);
    path.ancestors()
        .find_map(|ancestor| {
            let real = ancestor.canonicalize().ok()?;
            let rest = path.strip_prefix(ancestor).ok()?;
            Some(real.join(rest))
        })
        .unwrap_or(path)
}

fn in_git_hooks_or_config(path: &Path) -> bool {
    let names: Vec<_> = path.components().collect();
    names.windows(2).any(|pair| {
        pair[0] == Component::Normal(".git".as_ref())
            && [
                Component::Normal("hooks".as_ref()),
                Component::Normal("config".as_ref()),
            ]
            .contains(&pair[1])
    })
}

pub(crate) fn sandbox_paths<'a>(entries: impl Iterator<Item = &'a SandboxEntry>) -> SandboxPaths {
    let mut paths = SandboxPaths::default();
    for entry in entries {
        let list = match entry.list {
            SandboxList::Readable => &mut paths.readable,
            SandboxList::Writable => &mut paths.writable,
            SandboxList::Hidden => &mut paths.hidden,
        };
        list.push(entry.path.clone());
    }
    paths
}

fn normalized(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            other => normal.push(other),
        }
    }
    normal
}
