use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Subcommand;
use orch_protocol::{Fix, Reply, Request, StaleOverrides};

use crate::client::{self, ClientError};

#[derive(Subcommand)]
pub enum RepoCommand {
    Move { from: PathBuf, to: PathBuf },
    Forget { path: PathBuf },
}

pub fn run(command: RepoCommand) -> ExitCode {
    let result = match command {
        RepoCommand::Move { from, to } => move_repo(&from, &to),
        RepoCommand::Forget { path } => forget(&path),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => client::fail("repo", 1, err),
    }
}

fn move_repo(from: &Path, to: &Path) -> Result<(), ClientError> {
    let request = Request::MoveRepo {
        from: absolute(from),
        to: absolute(to),
    };
    let (repo, stale) = client::expect(request, |reply| match reply {
        Reply::RepoMoved {
            repo,
            stale_overrides,
        } => Some((repo, stale_overrides)),
        _ => None,
    })?;
    println!("Moved the Repo to {}", repo.display());
    warn_stale(&repo, stale);
    Ok(())
}

fn warn_stale(repo: &Path, stale: StaleOverrides) {
    if let Some(problem) = stale.unreadable {
        eprintln!(
            "warning: could not check your config for personal overrides of the old path: {problem}"
        );
    }
    let new_key = repo.display().to_string();
    for key in stale.keys {
        eprintln!(
            "warning: the personal override [repos.{key:?}] in your config still names the old path; rename it to [repos.{new_key:?}]"
        );
    }
}

fn forget(path: &Path) -> Result<(), ClientError> {
    let repo = absolute(path);
    let fix = Fix::ForgetRepo { repo: repo.clone() };
    client::request(Request::Fix { fix })?;
    println!("Forgot the Repo {}", repo.display());
    Ok(())
}

pub fn absolute(path: &Path) -> PathBuf {
    path.canonicalize()
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf())
}
