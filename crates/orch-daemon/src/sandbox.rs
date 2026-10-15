use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use orch_agent::{
    AgentAdapter, BWRAP, Placeholder, SandboxFailure, SandboxLayout, SandboxNeeds, SandboxPaths,
    sandbox_args, sandbox_carve_outs, sandbox_refusal,
};
use orch_config::xdg;
use orch_store::SessionRecord;

use crate::agents::default_adapter;
use crate::broker::{EnvVars, SHIMS_DIR, install_shims};
use crate::broker_sign::{IDENTITY_FILE, origin, public_key, user_identity};
use crate::private_dir;
use crate::state::Daemon;

const TMP_DIR: &str = "tmp";
const ALWAYS_HIDDEN: [&str; 3] = ["/mnt", "/media", "/root"];
const NESTED: [&str; 9] = [
    BWRAP,
    "--unshare-user",
    "--ro-bind",
    "/",
    "/",
    "--proc",
    "/proc",
    "--",
    "true",
];

pub type SandboxWarning = String;
pub type ProbeResult = Result<Option<SandboxWarning>, String>;

struct SessionPaths {
    worktree: PathBuf,
    tmp: PathBuf,
    subagent_worktrees: PathBuf,
    main_checkout: PathBuf,
    git_common_dir: PathBuf,
    holder_socket: PathBuf,
}

impl Daemon {
    pub(crate) fn sandbox_args(
        &self,
        record: &SessionRecord,
        repo: &Path,
        adapter: &dyn AgentAdapter,
        configured: &SandboxPaths,
    ) -> Result<(Vec<String>, EnvVars), String> {
        let paths = self.session_paths(record, repo, self.holder_socket(&record.id))?;
        let shims = self.session_dir(&record.id).join(SHIMS_DIR);
        let key = self
            .config
            .loader
            .global()
            .map_err(|err| err.to_string())?
            .signing_key;
        let public = key.as_deref().map(public_key).transpose()?;
        let identity = user_identity(repo)?;
        let identity_file = self.session_dir(&record.id).join(IDENTITY_FILE);
        let recorded = match &identity {
            Some(identity) => std::fs::write(&identity_file, identity.line()),
            None => std::fs::remove_file(&identity_file).or_else(|err| match err.kind() {
                io::ErrorKind::NotFound => Ok(()),
                _ => Err(err),
            }),
        };
        recorded.map_err(|err| format!("recording the Agent identity: {err}"))?;
        let origin = origin(repo)?;
        let env = install_shims(
            &shims,
            &self.config.orch_program,
            &origin,
            identity.as_ref(),
            public.as_deref(),
        )?;
        let mut layout = layout(paths, self.config.orch_program.clone(), configured)?;
        layout.read_only.push(shims);
        layout.masked.extend(key.filter(|key| key.exists()));
        let args = carved_args(&layout, &adapter.sandbox_needs(&xdg::process_env))?;
        Ok((args, env))
    }

    pub(crate) fn probe_sandbox(
        &self,
        record: Option<&SessionRecord>,
        repo: &Path,
        adapter: &dyn AgentAdapter,
        configured: &SandboxPaths,
    ) -> ProbeResult {
        let scratch = Scratch::new(&self.config.runtime_dir).map_err(|err| err.to_string())?;
        let paths = match record {
            Some(record) => self.session_paths(record, repo, scratch.socket.clone())?,
            None => SessionPaths {
                worktree: repo.to_path_buf(),
                main_checkout: repo.to_path_buf(),
                git_common_dir: git_common_dir(repo)?,
                ..scratch.paths()?
            },
        };
        let layout = layout(paths, self.config.orch_program.clone(), configured)?;
        probe(&carved_args(
            &layout,
            &adapter.sandbox_needs(&xdg::process_env),
        )?)
    }

    fn session_paths(
        &self,
        record: &SessionRecord,
        repo: &Path,
        holder_socket: PathBuf,
    ) -> Result<SessionPaths, String> {
        let tmp = self.session_dir(&record.id).join(TMP_DIR);
        let subagent_worktrees = orch_git::subagent_worktrees_dir(&record.worktree);
        private_dirs(&[&tmp, &subagent_worktrees])?;
        Ok(SessionPaths {
            worktree: record.worktree.clone(),
            tmp,
            subagent_worktrees,
            main_checkout: repo.to_path_buf(),
            git_common_dir: git_common_dir(repo)?,
            holder_socket,
        })
    }
}

pub fn probe_host_sandbox(orch_program: PathBuf) -> ProbeResult {
    let scratch = Scratch::new(&std::env::temp_dir()).map_err(|err| err.to_string())?;
    let layout = layout(scratch.paths()?, orch_program, &SandboxPaths::default())?;
    let mut needs = default_adapter().sandbox_needs(&xdg::process_env);
    needs.carve_outs.retain(|path| path.exists());
    probe(&carved_args(&layout, &needs)?)
}

fn carved_args(layout: &SandboxLayout, needs: &SandboxNeeds) -> Result<Vec<String>, String> {
    for (path, placeholder) in sandbox_carve_outs(layout, needs) {
        if path.symlink_metadata().is_ok() {
            continue;
        }
        let made = match placeholder {
            Placeholder::Dir => std::fs::create_dir_all(&path),
            Placeholder::File(content) => path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, content)),
        };
        made.map_err(|err| format!("creating {}: {err}", path.display()))?;
    }
    Ok(sandbox_args(layout, needs))
}

fn probe(args: &[String]) -> ProbeResult {
    run_bwrap(args, &["true"]).map_err(|failure| sandbox_refusal(&failure, sysctl))?;
    match run_bwrap(args, &NESTED) {
        Ok(()) => Ok(None),
        Err(failure) => Ok(Some(format!(
            "The Sandbox can't nest, so Claude's own Bash sandbox won't work here: {failure}"
        ))),
    }
}

fn run_bwrap(args: &[String], command: &[&str]) -> Result<(), SandboxFailure> {
    let output = Command::new(BWRAP)
        .args(args)
        .arg("--")
        .args(command)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    match output {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(SandboxFailure::Failed(
            String::from_utf8_lossy(&output.stderr).trim().into(),
        )),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err(SandboxFailure::Missing),
        Err(err) => Err(SandboxFailure::Failed(err.to_string())),
    }
}

fn sysctl(key: &str) -> Option<String> {
    std::fs::read_to_string(Path::new("/proc/sys").join(key.replace('.', "/"))).ok()
}

fn layout(
    paths: SessionPaths,
    orch_program: PathBuf,
    configured: &SandboxPaths,
) -> Result<SandboxLayout, String> {
    let home = xdg::process_env("HOME")
        .filter(|home| !home.is_empty())
        .map(|home| canonical(Path::new(&home)))
        .ok_or("the Sandbox needs HOME to be set")?;
    Ok(SandboxLayout {
        hidden: hidden(&home),
        home,
        tmp: paths.tmp,
        runtime_dir: xdg::runtime_dir(xdg::process_env),
        holder_socket: paths.holder_socket,
        worktree: paths.worktree,
        subagent_worktrees: paths.subagent_worktrees,
        main_checkout: paths.main_checkout,
        git_common_dir: paths.git_common_dir,
        read_only: vec![orch_program],
        masked: Vec::new(),
        configured: prepared_on_disk(configured)?,
    })
}

fn prepared_on_disk(configured: &SandboxPaths) -> Result<SandboxPaths, String> {
    let missing = configured.writable.iter().filter(|path| !path.exists());
    for dir in missing {
        std::fs::create_dir_all(dir).map_err(|err| format!("creating {}: {err}", dir.display()))?;
    }
    let mut hidden = Vec::new();
    for path in &configured.hidden {
        match path.metadata() {
            Ok(meta) if meta.is_dir() => hidden.push(path.clone()),
            Ok(_) => {
                return Err(format!(
                    "the Sandbox can only hide directories, not {}",
                    path.display()
                ));
            }
            Err(_) => {}
        }
    }
    Ok(SandboxPaths {
        hidden,
        ..configured.clone()
    })
}

fn git_common_dir(repo: &Path) -> Result<PathBuf, String> {
    orch_git::Repo::open(repo)
        .and_then(|git| git.common_dir())
        .map_err(|err| err.to_string())
}

fn private_dirs(dirs: &[&Path]) -> Result<(), String> {
    for dir in dirs {
        private_dir(dir).map_err(|err| format!("creating {}: {err}", dir.display()))?;
    }
    Ok(())
}

struct Scratch {
    dir: PathBuf,
    socket: PathBuf,
}

impl Scratch {
    fn new(parent: &Path) -> io::Result<Self> {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).map_err(|err| io::Error::other(err.to_string()))?;
        let name: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let dir = parent.join(format!("sandbox-probe-{name}"));
        private_dir(&dir)?;
        let socket = dir.join("socket");
        std::fs::write(&socket, "")?;
        Ok(Self { dir, socket })
    }

    fn paths(&self) -> Result<SessionPaths, String> {
        let [worktree, tmp, subagent_worktrees, git_common_dir] =
            ["worktree", "tmp", "subagents", "git"].map(|name| self.dir.join(name));
        private_dirs(&[&worktree, &tmp, &subagent_worktrees, &git_common_dir])?;
        Ok(SessionPaths {
            main_checkout: worktree.clone(),
            worktree,
            tmp,
            subagent_worktrees,
            git_common_dir,
            holder_socket: self.socket.clone(),
        })
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn hidden(home: &Path) -> Vec<PathBuf> {
    let other_homes = home
        .parent()
        .filter(|parent| *parent != Path::new("/"))
        .and_then(|parent| std::fs::read_dir(parent).ok())
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path());
    ALWAYS_HIDDEN
        .iter()
        .map(PathBuf::from)
        .chain(other_homes)
        .filter(|path| path.is_dir() && !home.starts_with(path))
        .collect()
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
