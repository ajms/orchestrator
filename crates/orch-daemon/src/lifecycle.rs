use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use orch_agent::{AgentAdapter, Argv, LaunchSpec, Preset};
use orch_config::{PresetError, RepoConfig, TrustHash, TrustItem, Untrusted};
use orch_core::{
    AgentState, ConversationId, Observation, PermissionMode, Phase, PhaseEvent, SessionId,
};
use orch_git::{SessionName, slugify};
use orch_holder::SESSION_ENV;
use orch_protocol::{CreateSession, Reply, RequestError, TrustNeeded};
use orch_store::{NewSession, RepoRoot, SessionRecord};

use crate::agents::{Adapter, adapter_for, default_adapter, default_program};
use crate::holder::Attach;
use crate::state::{Daemon, Live, gate_message};

pub(crate) const HOLDER_EXIT_WAIT: Duration = Duration::from_secs(5);
pub(crate) const PROMPT_FILE: &str = "prompt";
const PORT_BASE_ENV: &str = "ORCH_PORT_BASE";
const CLICK_QUEUE: usize = 64;

pub(crate) struct Busy(Arc<Daemon>);

impl Busy {
    pub(crate) fn new(daemon: &Arc<Daemon>) -> Self {
        daemon.lock().busy += 1;
        Self(daemon.clone())
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.lock().busy -= 1;
    }
}

pub(crate) async fn with_git<T, E>(
    repo: PathBuf,
    work: impl FnOnce(&orch_git::Repo) -> Result<T, E> + Send + 'static,
) -> Result<Result<T, E>, RequestError>
where
    T: Send + 'static,
    E: From<orch_git::Error> + Send + 'static,
{
    tokio::task::spawn_blocking(move || work(&orch_git::Repo::open(&repo)?))
        .await
        .map_err(refused)
}

pub(crate) fn refused(err: impl std::fmt::Display) -> RequestError {
    RequestError::Refused {
        message: err.to_string(),
    }
}

pub(crate) fn untrusted(repo: &Path, config: &RepoConfig) -> RequestError {
    match trust_needed(config) {
        Some(TrustNeeded { hash, items }) => RequestError::Untrusted {
            repo: repo.to_path_buf(),
            hash,
            items,
        },
        None => refused(Untrusted),
    }
}

pub(crate) fn trust_needed(config: &RepoConfig) -> Option<TrustNeeded> {
    config.trust_request().map(|request| TrustNeeded {
        hash: request.hash.as_str().into(),
        items: request.items.iter().map(describe).collect(),
    })
}

fn describe(item: &TrustItem) -> String {
    match item {
        TrustItem::SetupScript(script) => format!("Setup script: {script}"),
        TrustItem::TeardownScript(script) => format!("Teardown script: {script}"),
        TrustItem::Agent { binary, args } => {
            let program = binary.clone().unwrap_or_else(default_program);
            format!("Agent: {program} {}", args.join(" "))
        }
        TrustItem::Preset(preset) => format!("Preset {}", preset.name),
        TrustItem::DefaultPreset(preset) => format!("default Preset {}", preset.name),
    }
}

pub(crate) fn new_session_id() -> Result<SessionId, RequestError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(refused)?;
    let id = uuid::Builder::from_random_bytes(bytes).into_uuid();
    SessionId::parse(&id.to_string()).map_err(|_| refused("generated an invalid Session id"))
}

pub(crate) fn session_env(record: &SessionRecord) -> Vec<(String, String)> {
    let mut env = vec![
        (SESSION_ENV.into(), record.id.as_str().into()),
        (
            "ORCH_WORKTREE".into(),
            record.worktree.to_string_lossy().into_owned(),
        ),
    ];
    if let Some(block) = record.port_block {
        env.push((PORT_BASE_ENV.into(), block.base.to_string()));
    }
    env
}

impl Daemon {
    pub(crate) fn update(
        &self,
        id: &SessionId,
        change: impl FnOnce(&mut Live) -> Result<(), String>,
    ) -> Result<(), RequestError> {
        let mut state = self.lock();
        let live = state
            .sessions
            .get_mut(id)
            .ok_or(RequestError::UnknownSession)?;
        change(live).map_err(|message| RequestError::Refused { message })?;
        state.changed(id);
        Ok(())
    }

    pub(crate) fn snapshot(&self, id: &SessionId) -> Option<(SessionRecord, PathBuf)> {
        let state = self.lock();
        let live = state.sessions.get(id)?;
        Some((live.record.clone(), live.repo.clone()))
    }

    pub(crate) async fn repo_config(&self, repo: &Path) -> Result<RepoConfig, RequestError> {
        let known = repo.to_path_buf();
        let approval = self
            .store
            .call(move |store| match store.repo_by_path(&known)? {
                Some(repo) => store.trust_approval(repo.id),
                None => Ok(None),
            })
            .await?
            .map_err(refused)?;
        let loader = self.config.loader.clone();
        let repo = repo.to_path_buf();
        tokio::task::spawn_blocking(move || loader.repo(&repo, approval.as_ref()))
            .await
            .map_err(refused)?
            .map_err(refused)
    }

    pub(crate) async fn approve_trust(
        &self,
        repo: PathBuf,
        hash: String,
    ) -> Result<Reply, RequestError> {
        let loader = self.config.loader.clone();
        let (root, request) = tokio::task::spawn_blocking(move || {
            let root = RepoRoot::resolve(&repo).map_err(refused)?;
            let config = loader.repo(root.path(), None).map_err(refused)?;
            Ok::<_, RequestError>((root, config.trust_request()))
        })
        .await
        .map_err(refused)??;
        match request {
            Some(request) if request.hash.as_str() == hash => {}
            Some(_) => {
                return Err(refused(
                    "the Repo's config changed since it was shown; review it again",
                ));
            }
            None => return Err(refused("the Repo's config needs no Trust")),
        }
        self.store
            .call(move |store| {
                let repo = store.register_repo(&root)?;
                store.approve_trust(repo.id, &TrustHash::from_stored(hash))
            })
            .await?
            .map_err(refused)?;
        Ok(Reply::Done)
    }

    pub(crate) async fn create_session(
        self: &Arc<Self>,
        create: CreateSession,
    ) -> Result<Reply, RequestError> {
        let _busy = Busy::new(self);
        let inside = create.repo.clone();
        let root = tokio::task::spawn_blocking(move || RepoRoot::resolve(&inside))
            .await
            .map_err(refused)?
            .map_err(refused)?;
        let _guard = self.repo_guard(root.path()).await;
        let daemon = self.clone();
        let live = tokio::task::spawn_blocking(move || daemon.prepare(create))
            .await
            .map_err(refused)??;
        let session = live.record.id.clone();
        self.lock().insert(live);
        tokio::spawn(self.clone().run_setup(session.clone()));
        Ok(Reply::Created { session })
    }

    fn prepare(&self, create: CreateSession) -> Result<Live, RequestError> {
        let root = RepoRoot::resolve(&create.repo).map_err(refused)?;
        let registered = root.clone();
        let repo = self
            .store
            .call_blocking(move |store| store.register_repo(&registered))?
            .map_err(refused)?;
        let approval = self
            .store
            .call_blocking(move |store| store.trust_approval(repo.id))?
            .map_err(refused)?;
        let config = self
            .config
            .loader
            .repo(root.path(), approval.as_ref())
            .map_err(refused)?;
        let preset = select_preset(root.path(), &config, create.preset.as_deref())?;
        if config.setup_script().is_err() {
            return Err(untrusted(root.path(), &config));
        }
        let agent = config
            .agent()
            .map_err(|_| untrusted(root.path(), &config))?;
        let adapter = adapter_for(agent).map_err(refused)?;
        let global = self.config.loader.global().map_err(refused)?;
        let git = orch_git::Repo::open(root.path()).map_err(refused)?;
        let base = match create.base {
            Some(base) => base,
            None => git.default_base(config.base_branch()).map_err(refused)?,
        };
        let name = match create.branch {
            Some(branch) => {
                if git.branch_exists(&branch) {
                    return Err(refused(format!("branch {branch} already exists")));
                }
                let wanted = branch
                    .strip_prefix(&global.branch_prefix)
                    .unwrap_or(&branch);
                let slug = git
                    .unique_name(&global.branch_prefix, &slugify(wanted))
                    .slug;
                SessionName { slug, branch }
            }
            None => git.unique_name(&global.branch_prefix, &slugify(&create.prompt)),
        };
        let id = new_session_id()?;
        let worktree = git.create_worktree(&name, &base).map_err(refused)?;
        let new = NewSession {
            id: id.clone(),
            repo: repo.id,
            slug: name.slug,
            branch: name.branch,
            base,
            worktree: worktree.path.clone(),
            phase: Phase::SettingUp,
            preset: preset.name,
        };
        let ports = global.ports;
        let stored = self.store.call_blocking(move |store| {
            let created = store.create_session(new)?;
            match store.allocate_port_block(&created.id, &ports) {
                Ok(_) => store
                    .session(&created.id)
                    .map(|found| found.unwrap_or(created)),
                Err(err) => {
                    let _ = store.delete_session(&created.id);
                    Err(err)
                }
            }
        });
        let record = match stored {
            Ok(Ok(record)) => record,
            Ok(Err(err)) => {
                let _ = git.remove_session_worktree(&worktree, None);
                return Err(refused(err));
            }
            Err(err) => {
                let _ = git.remove_session_worktree(&worktree, None);
                return Err(err);
            }
        };
        let dir = self.session_dir(&id);
        let written = std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(dir.join(PROMPT_FILE), &create.prompt));
        if let Err(err) = written {
            eprintln!("orch daemon: storing the prompt of {}: {err}", id.as_str());
        }
        Ok(Live::new(record, repo.path, adapter))
    }

    pub(crate) async fn check_launch(&self, repo: &Path, preset: &str) -> Result<(), RequestError> {
        let config = self.repo_config(repo).await?;
        config.agent().map_err(|_| untrusted(repo, &config))?;
        select_preset(repo, &config, Some(preset)).map(drop)
    }

    pub(crate) async fn check_teardown(&self, repo: &Path) -> Result<(), RequestError> {
        let config = self.repo_config(repo).await?;
        config
            .teardown_script()
            .map(drop)
            .map_err(|_| untrusted(repo, &config))
    }

    fn session_repo(&self, id: &SessionId) -> Result<(SessionRecord, PathBuf), RequestError> {
        self.snapshot(id).ok_or(RequestError::UnknownSession)
    }

    pub(crate) async fn retry_setup(
        self: &Arc<Self>,
        id: &SessionId,
    ) -> Result<Reply, RequestError> {
        let (_, repo) = self.session_repo(id)?;
        let config = self.repo_config(&repo).await?;
        config
            .setup_script()
            .map_err(|_| untrusted(&repo, &config))?;
        self.update(id, |live| {
            live.transition(PhaseEvent::SetupRetried)?;
            live.setup_output = None;
            Ok(())
        })?;
        tokio::spawn(self.clone().run_setup(id.clone()));
        Ok(Reply::Done)
    }

    pub(crate) async fn start_anyway(
        self: &Arc<Self>,
        id: &SessionId,
    ) -> Result<Reply, RequestError> {
        let (record, repo) = self.session_repo(id)?;
        self.check_launch(&repo, &record.preset).await?;
        self.update(id, |live| {
            live.transition(PhaseEvent::SetupSkipped)?;
            live.launching = true;
            Ok(())
        })?;
        let daemon = self.clone();
        let id = id.clone();
        tokio::spawn(async move { daemon.launch_agent(&id, false).await });
        Ok(Reply::Done)
    }

    pub(crate) async fn resume(self: &Arc<Self>, id: &SessionId) -> Result<Reply, RequestError> {
        let _busy = Busy::new(self);
        let (record, repo) = self.session_repo(id)?;
        self.check_launch(&repo, &record.preset).await?;
        let mut replaced = None;
        self.update(id, |live| {
            if live.status.flags().worktree_missing {
                return Err("the Worktree is missing; recreate it first".into());
            }
            if live.status.phase() == Phase::Suspended {
                live.transition(PhaseEvent::Resumed)?;
                live.launching = true;
                return Ok(());
            }
            let finished = matches!(
                live.status.agent_state(),
                None | Some(AgentState::Exited | AgentState::Errored)
            );
            if !live.status.phase().is_live() || !finished {
                return Err("only a Suspended Session or a finished Agent can be resumed".into());
            }
            replaced = live.replace_holder(None).1;
            live.last_error = None;
            live.launching = true;
            Ok(())
        })?;
        if let Some(link) = replaced {
            self.release_holder(link, HOLDER_EXIT_WAIT).await;
        }
        self.launch_agent(id, true).await;
        Ok(Reply::Done)
    }

    pub(crate) async fn set_preset(
        self: &Arc<Self>,
        id: &SessionId,
        name: String,
    ) -> Result<Reply, RequestError> {
        let _busy = Busy::new(self);
        let (_claim, _, repo) = self.claim(id, |live| {
            live.status
                .check_preset_change()
                .map_err(|refusal| gate_message("Changing the Preset", refusal))
        })?;
        let config = self.repo_config(&repo).await?;
        let preset = select_preset(&repo, &config, Some(&name))?;
        config.agent().map_err(|_| untrusted(&repo, &config))?;
        let mut restart = false;
        let mut replaced = None;
        self.update(id, |live| {
            live.record.preset = preset.name;
            live.record.last_mode = None;
            live.status.forget_permission_mode();
            restart = live.status.phase().is_live();
            if restart {
                replaced = live.replace_holder(None).1;
                live.last_error = None;
                live.launching = true;
            }
            Ok(())
        })?;
        if let Some(link) = replaced {
            self.release_holder(link, HOLDER_EXIT_WAIT).await;
        }
        if restart {
            self.launch_agent(id, true).await;
        }
        Ok(Reply::Done)
    }

    pub(crate) fn set_muted(&self, id: &SessionId, muted: bool) -> Result<Reply, RequestError> {
        self.update(id, |live| {
            live.status.set_muted(muted);
            Ok(())
        })?;
        if muted {
            self.lock().dismiss(id);
        }
        Ok(Reply::Done)
    }

    pub(crate) async fn launch_agent(self: &Arc<Self>, id: &SessionId, resume: bool) {
        let _busy = Busy::new(self);
        self.set_launching(id, true);
        self.launch(id, resume).await;
        self.set_launching(id, false);
    }

    fn set_launching(&self, id: &SessionId, launching: bool) {
        if let Some(live) = self.lock().sessions.get_mut(id) {
            live.launching = launching;
        }
    }

    async fn launch(self: &Arc<Self>, id: &SessionId, resume: bool) {
        let Some((record, repo)) = self.snapshot(id) else {
            return;
        };
        let prompt_file = self.session_dir(id).join(PROMPT_FILE);
        let prompt = tokio::task::spawn_blocking(move || std::fs::read_to_string(prompt_file).ok())
            .await
            .ok()
            .flatten();
        let launch = match self.repo_config(&repo).await {
            Ok(config) => self.agent_argv(&repo, &config, &record, prompt, resume),
            Err(err) => Err(err.to_string()),
        };
        let spawned = match launch {
            Ok((adapter, argv)) => {
                let _ = self.update(id, |live| {
                    live.adapter = adapter;
                    Ok(())
                });
                self.spawn_holder(&record, argv).await
            }
            Err(message) => Err(message),
        };
        let attached = match spawned {
            Ok(()) => self
                .attach(id, Attach::Fresh)
                .await
                .map_err(|err| format!("cannot reach the Holder: {err}")),
            Err(message) => Err(message),
        };
        if let Err(message) = attached {
            self.agent_failed(id, message);
        }
    }

    fn agent_failed(&self, id: &SessionId, message: String) {
        eprintln!("orch daemon: Session {}: {message}", id.as_str());
        let _ = self.update(id, |live| {
            let now = std::time::Instant::now();
            live.status.observe(Observation::Spawned, now);
            live.status.observe(Observation::Exited { code: None }, now);
            live.last_error = Some(message);
            Ok(())
        });
    }

    fn agent_argv(
        &self,
        repo: &Path,
        config: &RepoConfig,
        record: &SessionRecord,
        prompt: Option<String>,
        resume: bool,
    ) -> Result<(Adapter, Vec<String>), String> {
        let agent = config.agent().map_err(|err| err.to_string())?;
        let adapter = adapter_for(agent)?;
        let preset =
            select_preset(repo, config, Some(&record.preset)).map_err(|err| err.to_string())?;
        let mut spec = LaunchSpec::new(
            record.id.clone(),
            self.config.orch_program.to_string_lossy(),
            preset,
        )
        .with_worktree(&record.worktree);
        if let Some(prompt) = prompt.filter(|prompt| !prompt.trim().is_empty()) {
            spec = spec.with_prompt(prompt);
        }
        let resume = match (resume, record.latest_conversation()) {
            (true, Some(conversation)) => Some((conversation, record.last_mode)),
            _ => None,
        };
        let argv = agent_command(adapter.as_ref(), &agent.args, spec, resume);
        Ok((adapter, argv))
    }

    async fn spawn_holder(&self, record: &SessionRecord, argv: Vec<String>) -> Result<(), String> {
        let mut command = crate::subprocess::command(&self.config.orch_program);
        command
            .arg("hold")
            .arg("--session")
            .arg(record.id.as_str())
            .arg("--runtime-dir")
            .arg(&self.config.runtime_dir)
            .arg("--cwd")
            .arg(&record.worktree)
            .arg("--base")
            .arg(&record.base);
        if let Some(block) = record.port_block {
            command
                .arg("--port-base")
                .arg(block.base.to_string())
                .arg("--port-size")
                .arg(block.size.to_string());
        }
        let display = self.lock().display.pairs();
        for (key, value) in session_env(record).into_iter().chain(display) {
            command.arg("--env").arg(format!("{key}={value}"));
        }
        let size = self
            .lock()
            .sessions
            .get(&record.id)
            .and_then(|live| live.panes.wanted());
        if let Some(size) = size {
            command
                .arg("--rows")
                .arg(size.rows.to_string())
                .arg("--cols")
                .arg(size.cols.to_string());
        }
        command
            .arg("--")
            .args(argv)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = command
            .output()
            .await
            .map_err(|err| format!("cannot start orch hold: {err}"))?;
        match output.status.success() {
            true => Ok(()),
            false => Err(format!(
                "orch hold failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )),
        }
    }

    pub(crate) async fn start(
        config: crate::DaemonConfig,
        store: crate::store::StoreHandle,
    ) -> Arc<Self> {
        let (clicks, clicked) = tokio::sync::mpsc::channel(CLICK_QUEUE);
        let daemon = Arc::new(Daemon::new(config, store, clicks));
        tokio::spawn(daemon.clone().route_clicks(clicked));
        for (id, phase) in daemon.load_sessions().await {
            match phase {
                Phase::Active | Phase::PrOpen => {
                    daemon.adopt_or_suspend(&id).await;
                }
                Phase::SettingUp => daemon.interrupted_setup(&id).await,
                _ => {}
            }
        }
        daemon
    }

    pub(crate) async fn adopt_or_suspend(self: &Arc<Self>, id: &SessionId) -> bool {
        if self.attach(id, Attach::Adopt).await.is_ok() {
            return false;
        }
        let mut suspended = false;
        let _ = self.update(id, |live| {
            if live.has_holder()
                || live.launching
                || live.exclusive
                || !live.status.phase().is_live()
            {
                return Ok(());
            }
            live.replace_holder(None);
            suspended = live.transition(PhaseEvent::Suspended).is_ok();
            Ok(())
        });
        suspended
    }

    pub(crate) async fn repo_adapter(&self, repo: &Path) -> Adapter {
        match self.repo_config(repo).await {
            Ok(config) => config
                .agent()
                .ok()
                .and_then(|agent| adapter_for(agent).ok())
                .unwrap_or_else(default_adapter),
            Err(_) => default_adapter(),
        }
    }

    async fn load_sessions(&self) -> Vec<(SessionId, Phase)> {
        let listed = self
            .store
            .call(|store| {
                let sessions = store.sessions()?;
                let repos = store.repos()?;
                Ok::<_, orch_store::StoreError>((sessions, repos))
            })
            .await;
        let Ok(Ok((sessions, repos))) = listed else {
            eprintln!("orch daemon: cannot read the Sessions from the state database");
            return Vec::new();
        };
        let mut known = Vec::new();
        for record in sessions.into_iter().filter(|r| !r.phase.is_terminal()) {
            let Some(repo) = repos.iter().find(|repo| repo.id == record.repo) else {
                continue;
            };
            let adapter = self.repo_adapter(&repo.path).await;
            let id = record.id.clone();
            let mut live = Live::new(record, repo.path.clone(), adapter);
            live.setup_output = self.read_setup_log(&id).await;
            known.push((id, live.status.phase()));
            self.lock().insert(live);
        }
        known
    }
}

pub(crate) fn select_preset(
    repo: &Path,
    config: &RepoConfig,
    name: Option<&str>,
) -> Result<Preset, RequestError> {
    config.select_preset(name).map_err(|err| match err {
        PresetError::Unknown(name) => refused(format!("unknown Preset {name}")),
        PresetError::Untrusted(_) => untrusted(repo, config),
    })
}

pub(crate) fn agent_command(
    adapter: &dyn AgentAdapter,
    agent_args: &[String],
    spec: LaunchSpec,
    resume: Option<(&ConversationId, Option<PermissionMode>)>,
) -> Vec<String> {
    let spec = LaunchSpec {
        preset: adapter.capabilities().effective_preset(spec.preset.clone()),
        ..spec
    };
    let Argv { program, args } = match resume {
        Some((conversation, mode)) => adapter.restart(&spec, Some(conversation), mode),
        None => adapter.launch(&spec),
    };
    std::iter::once(program)
        .chain(agent_args.iter().cloned())
        .chain(args)
        .collect()
}

#[cfg(test)]
mod tests {
    use orch_agent::{Capabilities, Presets};

    use super::*;

    struct PresetEcho(Capabilities);

    impl AgentAdapter for PresetEcho {
        fn capabilities(&self) -> Capabilities {
            self.0
        }

        fn launch(&self, spec: &LaunchSpec) -> Argv {
            Argv {
                program: "agent".into(),
                args: vec![spec.preset.name.clone()],
            }
        }
    }

    fn launched_preset(capabilities: Capabilities) -> String {
        let plan = Presets::default().get("plan").unwrap().clone();
        let spec = LaunchSpec::new(SessionId("s".into()), "orch", plan);
        let argv = agent_command(&PresetEcho(capabilities), &[], spec, None);
        argv.last().unwrap().clone()
    }

    #[test]
    fn an_agent_without_modes_is_launched_with_the_inherit_preset() {
        assert_eq!(launched_preset(Capabilities::default()), "inherit");
    }

    #[test]
    fn an_agent_with_modes_is_launched_with_the_chosen_preset() {
        let modes = Capabilities {
            modes: true,
            ..Capabilities::default()
        };
        assert_eq!(launched_preset(modes), "plan");
    }
}
