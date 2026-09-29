use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::error::{ConfigError, ConfigProblem};
use crate::global::{GlobalConfig, GlobalFile};
use crate::repo::{RepoConfig, RepoLayer, Source};
use crate::trust::TrustHash;
use crate::xdg;

pub const REPO_FILE: &str = ".orchestrator.toml";

#[derive(Debug, Clone)]
pub struct ConfigLoader {
    global_path: PathBuf,
    home: Option<PathBuf>,
}

impl ConfigLoader {
    pub fn new(global_path: impl Into<PathBuf>) -> Self {
        Self {
            global_path: global_path.into(),
            home: None,
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
        let loader = Self::new(base.join("orchestrator").join("config.toml"));
        Some(match xdg::home(&lookup) {
            Some(home) => loader.with_home(home),
            None => loader,
        })
    }

    pub fn global_path(&self) -> &Path {
        &self.global_path
    }

    pub fn global(&self) -> Result<GlobalConfig, ConfigError> {
        Ok(self.global_file()?.resolve())
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
        let expanded = match (key.strip_prefix("~/"), &self.home) {
            (Some(rest), Some(home)) => home.join(rest),
            _ => PathBuf::from(key),
        };
        canonical(&expanded)
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

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
