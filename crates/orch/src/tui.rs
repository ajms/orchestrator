use std::io::IsTerminal;
use std::path::Path;
use std::process::ExitCode;

use orch_holder::default_runtime_dir;
use orch_protocol::DisplayVars;
use orch_tui::{Options, TuiConfig};

use crate::client::{self, ClientError};

const NO_TERMINAL: u8 = 2;

pub fn run() -> ExitCode {
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        eprintln!(
            "orch: the TUI needs a terminal; run `orch` in an interactive terminal, or see `orch --help` for the subcommands"
        );
        return ExitCode::from(NO_TERMINAL);
    }
    match start() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("orch: {message}");
            ExitCode::FAILURE
        }
    }
}

fn start() -> Result<(), String> {
    let orch_program = client::orch_program().map_err(|err| err.to_string())?;
    let cwd = std::env::current_dir().ok();
    let options = Options {
        runtime_dir: default_runtime_dir(),
        orch_program,
        config: TuiConfig {
            display: DisplayVars::from_env(),
            ..tui_config(cwd.as_deref())
        },
    };
    client::block_on(orch_tui::run(options))
        .map_err(|err: ClientError| err.to_string())?
        .map_err(|err| err.to_string())
}

pub fn tui_config(cwd: Option<&Path>) -> TuiConfig {
    let cwd_repo = cwd
        .and_then(|cwd| orch_git::Repo::open(cwd).ok())
        .map(|repo| repo.root().to_path_buf());
    TuiConfig {
        cwd_repo,
        ..TuiConfig::default()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::Command;

    use tempfile::TempDir;

    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    struct Fixture {
        dir: TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
            }
        }

        fn repo(&self, name: &str) -> PathBuf {
            let root = self.dir.path().join(name);
            std::fs::create_dir_all(&root).unwrap();
            git(&root, &["init", "-q", "-b", "main"]);
            git(&root, &["commit", "-q", "--allow-empty", "-m", "init"]);
            root.canonicalize().unwrap()
        }
    }

    #[test]
    fn launching_inside_a_repo_preselects_its_main_checkout() {
        let fx = Fixture::new();
        let repo = fx.repo("app");
        std::fs::create_dir_all(repo.join("src/deep")).unwrap();

        let config = tui_config(Some(&repo.join("src/deep")));

        assert_eq!(config.cwd_repo, Some(repo));
    }

    #[test]
    fn launching_inside_a_worktree_preselects_the_repo_it_belongs_to() {
        let fx = Fixture::new();
        let repo = fx.repo("app");
        let worktree = repo.join(".orchestrator/worktrees/fix");
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "orch/fix",
                worktree.to_str().unwrap(),
            ],
        );

        let config = tui_config(Some(&worktree));

        assert_eq!(config.cwd_repo, Some(repo));
    }

    #[test]
    fn launching_outside_any_repo_preselects_nothing() {
        let fx = Fixture::new();

        let config = tui_config(Some(fx.dir.path()));

        assert_eq!(config.cwd_repo, None);
    }
}
