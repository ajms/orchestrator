use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use orch_agent::{AgentAdapter, SandboxFailure, SandboxLayout, sandbox_policy, sandbox_refusal};
use orch_config::xdg;
use orch_store::SessionRecord;

use crate::agents::default_adapter;
use crate::state::Daemon;

const TMP_DIR: &str = "tmp";
const ALWAYS_HIDDEN: [&str; 3] = ["/mnt", "/media", "/root"];
const BWRAP: &str = "bwrap";
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

pub type ProbeResult = Result<Option<String>, String>;

struct SessionPaths {
    worktree: PathBuf,
    tmp: PathBuf,
    subagent_worktrees: PathBuf,
    main_checkout: PathBuf,
    git_common_dir: PathBuf,
    holder_socket: PathBuf,
}

impl Daemon {
    pub(crate) fn sandbox_policy(
        &self,
        record: &SessionRecord,
        repo: &Path,
        adapter: &dyn AgentAdapter,
    ) -> Result<Vec<String>, String> {
        let paths = self.session_paths(record, repo, self.holder_socket(&record.id))?;
        let layout = layout(paths, self.config.orch_program.clone())?;
        Ok(sandbox_policy(
            &layout,
            &adapter.sandbox_needs(&xdg::process_env),
        ))
    }

    pub(crate) fn probe_sandbox(
        &self,
        record: Option<&SessionRecord>,
        repo: &Path,
        adapter: &dyn AgentAdapter,
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
        let layout = layout(paths, self.config.orch_program.clone())?;
        probe(&sandbox_policy(
            &layout,
            &adapter.sandbox_needs(&xdg::process_env),
        ))
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
    let layout = layout(scratch.paths()?, orch_program)?;
    let needs = default_adapter().sandbox_needs(&xdg::process_env);
    probe(&sandbox_policy(&layout, &needs))
}

fn probe(policy: &[String]) -> ProbeResult {
    run_bwrap(policy, &["true"]).map_err(|failure| sandbox_refusal(&failure, sysctl))?;
    match run_bwrap(policy, &NESTED) {
        Ok(()) => Ok(None),
        Err(failure) => {
            let said = match failure {
                SandboxFailure::Missing => "bwrap is not installed".into(),
                SandboxFailure::Failed(stderr) => stderr,
            };
            Ok(Some(format!(
                "The Sandbox can't nest, so Claude's own Bash sandbox won't work here: {said}"
            )))
        }
    }
}

fn run_bwrap(policy: &[String], command: &[&str]) -> Result<(), SandboxFailure> {
    let output = Command::new(BWRAP)
        .args(policy)
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

fn layout(paths: SessionPaths, orch_program: PathBuf) -> Result<SandboxLayout, String> {
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
    })
}

fn git_common_dir(repo: &Path) -> Result<PathBuf, String> {
    orch_git::Repo::open(repo)
        .and_then(|git| git.common_dir())
        .map_err(|err| err.to_string())
}

fn private_dirs(dirs: &[&Path]) -> Result<(), String> {
    for dir in dirs {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|err| format!("creating {}: {err}", dir.display()))?;
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
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)?;
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
