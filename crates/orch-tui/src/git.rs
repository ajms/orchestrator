use orch_git::{Repo, SessionName, SessionWorktree};

use crate::event::{ReviewData, ReviewTarget};
use crate::review::parse_unified_diff;

pub fn load_review(target: &ReviewTarget) -> Result<ReviewData, orch_git::Error> {
    let repo = Repo::open(&target.repo)?;
    let worktree = SessionWorktree {
        path: target.worktree.clone(),
        name: SessionName {
            slug: target.slug.clone(),
            branch: target.branch.clone(),
        },
        base: target.base.clone(),
        existing_branch: false,
    };
    if let Some(name) = &target.subagent {
        let Some((base, head)) = repo.subagent_diff_refs(&target.worktree, name)? else {
            return Err(orch_git::Error::Io {
                context: format!("Subagent {name}"),
                message: "no recorded base/head to diff".into(),
            });
        };
        let diff = repo.diff(&base, &head)?;
        return Ok(ReviewData {
            merge_base: base,
            tree: head,
            files: parse_unified_diff(&diff),
        });
    }
    let snapshot = repo.review_snapshot(&worktree)?;
    let diff = repo.diff(&snapshot.merge_base, &snapshot.tree)?;
    Ok(ReviewData {
        merge_base: snapshot.merge_base,
        tree: snapshot.tree,
        files: parse_unified_diff(&diff),
    })
}
