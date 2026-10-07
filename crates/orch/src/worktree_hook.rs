use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

use orch_agent::{AgentAdapter, ClaudeCode, WorktreeRequest};
use orch_git::Repo;

pub fn run(worktree: &Path) -> ExitCode {
    let mut payload = String::new();
    if let Err(err) = std::io::stdin().read_to_string(&mut payload) {
        eprintln!("orch worktree-hook: {err}");
        return ExitCode::FAILURE;
    }
    let Some(request) = ClaudeCode::default().worktree_request(&payload) else {
        eprintln!("orch worktree-hook: not a worktree hook payload");
        return ExitCode::FAILURE;
    };
    let done = Repo::open(worktree).and_then(|repo| match request {
        WorktreeRequest::Create { name } => repo
            .create_subagent_worktree(worktree, &name)
            .map(|path| println!("{}", path.display())),
        WorktreeRequest::Remove { path } => repo.remove_subagent_worktree(worktree, &path),
    });
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("orch worktree-hook: {err}");
            ExitCode::FAILURE
        }
    }
}
