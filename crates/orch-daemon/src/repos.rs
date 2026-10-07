use std::path::{Path, PathBuf};
use std::sync::Arc;

use orch_agent::built_in_names;
use orch_config::{PresetError, RepoConfig};
use orch_core::SessionId;
use orch_protocol::{
    AgentChoice, DefaultPreset, PresetChoice, Reply, RepoSettings, RequestError, StaleOverrides,
};
use orch_store::RepoRoot;

use crate::agents::{installed_adapter, modes};
use crate::lifecycle::{Busy, refused, trust_needed, with_git};
use crate::reconcile::Pass;
use crate::state::Daemon;

impl Daemon {
    pub(crate) async fn repos(&self) -> Result<Reply, RequestError> {
        let repos = self
            .store
            .call(|store| store.repos())
            .await?
            .map_err(refused)?;
        Ok(Reply::Repos {
            repos: repos
                .into_iter()
                .filter(|repo| !repo.missing)
                .map(|repo| repo.path)
                .collect(),
        })
    }

    pub(crate) async fn repo_settings(&self, repo: PathBuf) -> Result<Reply, RequestError> {
        let root = tokio::task::spawn_blocking(move || RepoRoot::resolve(&repo))
            .await
            .map_err(refused)?
            .map_err(refused)?;
        let repo = root.path().to_path_buf();
        let config = self.repo_config(&repo).await?;
        let default_base = self.default_base(&repo, &config).await.ok();
        let trust = match config.is_trusted() {
            true => None,
            false => trust_needed(&config),
        };
        Ok(Reply::RepoSettings(RepoSettings {
            agents: agent_choices(&config, &repo),
            default_agent: config.default_agent().into(),
            default_base,
            review_command: config.review_command().map(String::from),
            branch_prefix: self.branch_prefix().await,
            trust,
            repo,
        }))
    }

    pub(crate) async fn move_repo(
        self: &Arc<Self>,
        from: PathBuf,
        to: PathBuf,
    ) -> Result<Reply, RequestError> {
        let _busy = Busy::new(self);
        let registered = self.registered_repo_at(&from).await?;
        let from = registered.path.clone();
        let root = tokio::task::spawn_blocking(move || RepoRoot::resolve(&to))
            .await
            .map_err(refused)?
            .map_err(refused)?;
        let to = root.path().to_path_buf();
        let _guards = self.repo_guards([&from, &to]).await;
        if from.exists()
            && self
                .lock()
                .sessions
                .values()
                .any(|live| live.repo == from && live.is_running())
        {
            return Err(refused(
                "the Repo has running Agents and is still at its old path; quit them before moving it",
            ));
        }
        let id = registered.id;
        let records = self
            .store
            .call(move |store| store.sessions_in(id))
            .await?
            .map_err(refused)?;
        let relocated: Vec<(SessionId, PathBuf)> = records
            .into_iter()
            .filter(|record| !record.phase.is_terminal())
            .filter_map(|record| {
                let rest = record.worktree.strip_prefix(&from).ok()?;
                Some((record.id, to.join(rest)))
            })
            .collect();
        let worktrees: Vec<PathBuf> = relocated
            .iter()
            .map(|(_, worktree)| worktree.clone())
            .filter(|worktree| worktree.is_dir())
            .collect();
        with_git(to.clone(), move |git| git.repair_worktrees(&worktrees))
            .await?
            .map_err(|err| {
                refused(format!(
                    "repairing the Worktrees failed, nothing was moved: {err}"
                ))
            })?;
        self.store
            .call(move |store| store.move_repo(id, &root))
            .await?
            .map_err(refused)?;
        {
            let mut state = self.lock();
            for (session, worktree) in relocated {
                if let Some(live) = state.sessions.get_mut(&session) {
                    live.relocate(to.clone(), worktree);
                    state.changed(&session);
                }
            }
        }
        let stale_overrides = self.stale_overrides(from).await;
        let daemon = self.clone();
        tokio::spawn(async move { daemon.reconcile(Pass::Full).await });
        Ok(Reply::RepoMoved {
            repo: to,
            stale_overrides,
        })
    }

    async fn registered_repo_at(&self, path: &Path) -> Result<orch_store::Repo, RequestError> {
        let mut candidates = vec![path.to_path_buf()];
        candidates.extend(path.canonicalize().ok());
        if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
            candidates.extend(parent.canonicalize().ok().map(|parent| parent.join(name)));
        }
        self.store
            .call(move |store| {
                for candidate in candidates {
                    if let Some(repo) = store.repo_by_path(&candidate)? {
                        return Ok(Some(repo));
                    }
                }
                Ok::<_, orch_store::StoreError>(None)
            })
            .await?
            .map_err(refused)?
            .ok_or_else(|| refused("no such Repo"))
    }

    async fn stale_overrides(&self, from: PathBuf) -> StaleOverrides {
        let loader = self.config.loader.clone();
        let checked = tokio::task::spawn_blocking(move || loader.override_keys_for(&from))
            .await
            .map_err(|err| err.to_string())
            .and_then(|keys| keys.map_err(|err| err.to_string()));
        match checked {
            Ok(keys) => StaleOverrides {
                keys,
                unreadable: None,
            },
            Err(unreadable) => StaleOverrides {
                keys: Vec::new(),
                unreadable: Some(unreadable),
            },
        }
    }
}

fn agent_choices(config: &RepoConfig, repo: &Path) -> Vec<AgentChoice> {
    let mut names = built_in_names().collect::<Vec<_>>();
    if !names.contains(&config.default_agent()) {
        names.push(config.default_agent());
    }
    let presets = config.presets();
    names
        .into_iter()
        .map(|name| AgentChoice {
            name: name.into(),
            unavailable: config
                .agent(name)
                .ok()
                .and_then(|agent| installed_adapter(&agent, repo).err()),
            presets: presets
                .offered(modes(name))
                .map(|preset| PresetChoice {
                    name: preset.name.clone(),
                    lacks_rules: preset.lacks_rules(name),
                })
                .collect(),
            default_preset: default_preset(config, name),
        })
        .collect()
}

fn default_preset(config: &RepoConfig, agent: &str) -> Option<DefaultPreset> {
    let configured = config.default_preset()?;
    let name = match config.select_preset(None, modes(agent)) {
        Ok(preset) => preset.name,
        Err(
            PresetError::Unknown(name)
            | PresetError::Untrusted(name)
            | PresetError::Unsupported(name),
        ) => name,
    };
    let unsupported = (name != configured).then(|| configured.to_string());
    Some(DefaultPreset { name, unsupported })
}
