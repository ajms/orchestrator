use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use orch_agent::{AgentAdapter, SandboxLayout, sandbox_policy};
use orch_config::xdg;
use orch_store::SessionRecord;

use crate::state::Daemon;

const TMP_DIR: &str = "tmp";
const ALWAYS_HIDDEN: [&str; 3] = ["/mnt", "/media", "/root"];

impl Daemon {
    pub(crate) fn sandbox_policy(
        &self,
        record: &SessionRecord,
        repo: &Path,
        adapter: &dyn AgentAdapter,
    ) -> Result<Vec<String>, String> {
        let home = xdg::process_env("HOME")
            .filter(|home| !home.is_empty())
            .map(|home| canonical(Path::new(&home)))
            .ok_or("the Sandbox needs HOME to be set")?;
        let tmp = self.session_dir(&record.id).join(TMP_DIR);
        let subagent_worktrees = orch_git::subagent_worktrees_dir(&record.worktree);
        for dir in [&tmp, &subagent_worktrees] {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(|err| format!("creating {}: {err}", dir.display()))?;
        }
        let git_common_dir = orch_git::Repo::open(repo)
            .and_then(|git| git.common_dir())
            .map_err(|err| err.to_string())?;
        let layout = SandboxLayout {
            hidden: hidden(&home),
            home,
            tmp,
            runtime_dir: xdg::runtime_dir(xdg::process_env),
            holder_socket: self.holder_socket(&record.id),
            worktree: record.worktree.clone(),
            subagent_worktrees,
            main_checkout: repo.to_path_buf(),
            git_common_dir,
            read_only: vec![self.config.orch_program.clone()],
        };
        let needs = adapter.sandbox_needs(&xdg::process_env);
        Ok(sandbox_policy(&layout, &needs))
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
