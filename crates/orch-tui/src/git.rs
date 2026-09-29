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
    };
    let snapshot = repo.review_snapshot(&worktree)?;
    let diff = repo.diff(&snapshot.merge_base, &snapshot.tree)?;
    Ok(ReviewData {
        merge_base: snapshot.merge_base,
        tree: snapshot.tree,
        files: parse_unified_diff(&diff),
    })
}
