use std::io;
use std::path::{Path, PathBuf};

use orch_agent::{AgentAdapter, ClaudeCode};
use serde::de::DeserializeOwned;

use crate::error::{ConfigError, ConfigProblem};
use crate::global::{GlobalConfig, GlobalFile};
use crate::repo::{RepoConfig, RepoLayer, Source};
use crate::sandbox::SandboxContext;
use crate::trust::TrustHash;
use crate::xdg;

pub const REPO_FILE: &str = ".orchestrator.toml";

#[derive(Debug, Clone)]
pub struct ConfigLoader {
    global_path: PathBuf,
    home: Option<PathBuf>,
    state_dir: Option<PathBuf>,
    carve_outs: Vec<PathBuf>,
}

impl ConfigLoader {
    pub fn new(global_path: impl Into<PathBuf>) -> Self {
        Self {
            global_path: global_path.into(),
            home: None,
            state_dir: None,
            carve_outs: Vec::new(),
        }
    }

    pub fn with_home(self, home: impl Into<PathBuf>) -> Self {
        Self {
            home: Some(home.into()),
            ..self
        }
    }

    pub fn from_env() -> Option<Self> {
        Self::from_env_vars(xdg::process_env)
    }

    pub fn from_env_vars(lookup: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let base = xdg::config_home(&lookup)?;
        let loader = Self {
            state_dir: xdg::state_home(&lookup).map(|state| state.join("orchestrator")),
            carve_outs: ClaudeCode::default().sandbox_needs(&lookup).carve_outs,
            ..Self::new(base.join("orchestrator").join("config.toml"))
        };
        Some(match xdg::home(&lookup) {
            Some(home) => loader.with_home(home),
            None => loader,
        })
    }

    pub fn global_path(&self) -> &Path {
        &self.global_path
    }

    pub fn global(&self) -> Result<GlobalConfig, ConfigError> {
        Ok(self.global_file()?.resolve(self.home.as_deref()))
    }

    pub fn repo(
        &self,
        repo_root: &Path,
        approved: Option<&TrustHash>,
    ) -> Result<RepoConfig, ConfigError> {
        let global = self.global_file()?;
        let repo_path = repo_root.join(REPO_FILE);
        let repo_file: RepoLayer = read_toml(&repo_path)?;
        let empty = RepoLayer::default();
        let personal = self
            .matching_keys(&global, repo_root)
            .next()
            .and_then(|key| global.repos.get(key))
            .unwrap_or(&empty);
        RepoConfig::layered(
            &global.notifications,
            [
                (Source::GlobalDefault, &global.defaults),
                (Source::RepoFile, &repo_file),
                (Source::PersonalOverride, personal),
            ],
            &self.sandbox_context(),
            approved,
        )
        .map_err(|(source, problem)| ConfigError::Invalid {
            path: match source {
                Source::RepoFile => repo_path,
                Source::GlobalDefault | Source::PersonalOverride => self.global_path.clone(),
            },
            problem,
        })
    }

    pub fn override_keys_for(&self, repo_path: &Path) -> Result<Vec<String>, ConfigError> {
        let global = self.global_file()?;
        Ok(self.matching_keys(&global, repo_path).cloned().collect())
    }

    fn matching_keys<'a>(
        &'a self,
        global: &'a GlobalFile,
        repo: &Path,
    ) -> impl Iterator<Item = &'a String> {
        let target = canonical(repo);
        global
            .repos
            .keys()
            .filter(move |key| self.key_path(key) == target)
    }

    fn key_path(&self, key: &str) -> PathBuf {
        canonical(&expand_home(key, self.home.as_deref()))
    }

    fn sandbox_context(&self) -> SandboxContext {
        let config_dir = self.global_path.parent().map(Path::to_path_buf);
        SandboxContext {
            home: self.home.clone(),
            orch_dirs: config_dir
                .into_iter()
                .chain(self.state_dir.clone())
                .collect(),
            carve_outs: self.carve_outs.clone(),
        }
    }

    fn global_file(&self) -> Result<GlobalFile, ConfigError> {
        let file: GlobalFile = read_toml(&self.global_path)?;
        file.check().map_err(|problem| self.invalid(problem))?;
        Ok(file)
    }

    fn invalid(&self, problem: ConfigProblem) -> ConfigError {
        ConfigError::Invalid {
            path: self.global_path.clone(),
            problem,
        }
    }
}

fn read_toml<T: DeserializeOwned + Default>(path: &Path) -> Result<T, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(T::default()),
        Err(source) => {
            return Err(ConfigError::Io {
                path: path.into(),
                source,
            });
        }
    };
    toml::from_str(&text).map_err(|err| ConfigError::Parse {
        path: path.into(),
        message: err.to_string(),
    })
}

pub(crate) fn expand_home(path: &str, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
