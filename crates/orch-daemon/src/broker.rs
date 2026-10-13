use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use orch_agent::{GuardAnswer, GuardHit, GuardKind};
use orch_core::{Observation, PermissionMode, SessionId};
use orch_holder::{BrokerReply, BrokerRequest, GuardId, RefUpdate, RemoteRef, ToHolder};
use orch_store::SessionRecord;
use tokio::sync::oneshot;

use crate::lifecycle::{select_preset, with_git};
use crate::state::{Daemon, PendingGuard};

pub(crate) type EnvVars = Vec<(String, String)>;

pub(crate) const SHIMS_DIR: &str = "shims";
const REMOTE_HELPER: &str = "git-remote-orch";
const PUSH_URL: &str = "orch::origin";
const DENIED: &str = "the user denied this push in the Orchestrator";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AllowedPush {
    pub(crate) commit: String,
    pub(crate) lease: Option<String>,
}

fn is_object_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn allowed_push(
    updates: &[RefUpdate],
    branch: &str,
    base: &str,
) -> Result<AllowedPush, String> {
    let mut refusals: Vec<String> = Vec::new();
    let mut allowed = None;
    for update in updates {
        match allowed_update(update, branch, base) {
            Ok(push) => allowed = allowed.or(Some(push)),
            Err(refusal) if !refusals.contains(&refusal) => refusals.push(refusal),
            Err(_) => {}
        }
    }
    match allowed {
        Some(push) if refusals.is_empty() => Ok(push),
        _ if refusals.is_empty() => Err("there is nothing to push".into()),
        _ => Err(refusals.join("; ")),
    }
}

fn allowed_update(update: &RefUpdate, branch: &str, base: &str) -> Result<AllowedPush, String> {
    let dst = update.dst.as_str();
    if dst == format!("refs/heads/{base}") {
        return Err(format!("the Broker never pushes the Base branch {base}"));
    }
    if let Some(tag) = dst.strip_prefix("refs/tags/") {
        return Err(format!("the Broker doesn't push tags ({tag})"));
    }
    if dst != format!("refs/heads/{branch}") {
        return Err(format!(
            "the Broker pushes only this Session's Branch {branch}, not {dst}"
        ));
    }
    let Some(commit) = update.commit.clone() else {
        return Err(match update.src.is_empty() {
            true => format!("the Broker doesn't delete {branch} on origin"),
            false => format!("{} is not a commit", update.src),
        });
    };
    if !is_object_id(&commit) {
        return Err(format!("{commit} is not a full commit id"));
    }
    if update.force && update.lease.is_none() {
        return Err("the Broker refuses a plain force push; use --force-with-lease".into());
    }
    if let Some(lease) = update.lease.as_deref()
        && !lease.is_empty()
        && !is_object_id(lease)
    {
        return Err(format!("the lease {lease} is not a full commit id"));
    }
    Ok(AllowedPush {
        commit,
        lease: update.lease.clone(),
    })
}

fn push_asks(mode: Option<PermissionMode>) -> bool {
    matches!(mode, Some(PermissionMode::Plan | PermissionMode::Default))
}

pub(crate) fn install_shims(dir: &Path, orch: &Path) -> Result<EnvVars, String> {
    let helper = dir.join(REMOTE_HELPER);
    let quoted = orch.to_string_lossy().replace('\'', r"'\''");
    let script = format!("#!/bin/sh\nexec '{quoted}' broker git-remote \"$@\"\n");
    std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&helper, script))
        .and_then(|()| std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o755)))
        .map_err(|err| format!("installing the Broker's git shim: {err}"))?;
    let path = match std::env::var("PATH") {
        Ok(path) if !path.is_empty() => format!("{}:{path}", dir.display()),
        _ => dir.display().to_string(),
    };
    let next: u32 = std::env::var("GIT_CONFIG_COUNT")
        .ok()
        .and_then(|count| count.parse().ok())
        .unwrap_or(0);
    Ok(vec![
        ("PATH".into(), path),
        ("GIT_CONFIG_COUNT".into(), (next + 1).to_string()),
        (
            format!("GIT_CONFIG_KEY_{next}"),
            "remote.origin.pushurl".into(),
        ),
        (format!("GIT_CONFIG_VALUE_{next}"), PUSH_URL.into()),
    ])
}

impl Daemon {
    pub(crate) async fn broker(
        self: Arc<Self>,
        id: SessionId,
        broker: GuardId,
        request: BrokerRequest,
    ) {
        let reply = self
            .serve_broker(&id, broker, request)
            .await
            .unwrap_or_else(|reason| BrokerReply::Refused { reason });
        if let Some(live) = self.lock().sessions.get(&id) {
            live.send_to_holder(ToHolder::BrokerReply { id: broker, reply });
        }
    }

    async fn serve_broker(
        &self,
        id: &SessionId,
        broker: GuardId,
        request: BrokerRequest,
    ) -> Result<BrokerReply, String> {
        let (record, repo, sandboxed) = {
            let state = self.lock();
            let live = state.sessions.get(id).ok_or("the Session has ended")?;
            (live.record.clone(), live.repo.clone(), live.sandboxed)
        };
        if !sandboxed {
            return Err("the Broker acts only for sandboxed Sessions".into());
        }
        let branch = record.branch.clone();
        match request {
            BrokerRequest::Refs => {
                let listed = with_git(repo, move |git| git.branch_on_origin(&branch))
                    .await
                    .map_err(|err| err.to_string())?
                    .map_err(|err| err.to_string())?;
                let refs = listed.map(|commit| RemoteRef {
                    name: format!("refs/heads/{}", record.branch),
                    commit,
                });
                Ok(BrokerReply::Refs {
                    refs: refs.into_iter().collect(),
                })
            }
            BrokerRequest::Push { updates } => {
                let push = allowed_push(&updates, &record.branch, &record.base)?;
                if self.push_asks(&record, &repo).await {
                    self.ask_push(id, broker, &record.branch, push.lease.is_some())
                        .await?;
                }
                let leased = push.lease.is_some();
                with_git(repo, move |git| {
                    git.push_brokered(&branch, &push.commit, push.lease.as_deref())
                })
                .await
                .map_err(|err| err.to_string())?
                .map_err(|err| match err {
                    orch_git::Error::Git { stderr, .. } => {
                        format!("pushing failed: {}", stderr.trim())
                    }
                    other => format!("pushing failed: {other}"),
                })?;
                let how = if leased {
                    " with --force-with-lease"
                } else {
                    ""
                };
                Ok(BrokerReply::Done {
                    said: format!("pushed {} to origin{how}", record.branch),
                })
            }
        }
    }

    async fn push_asks(&self, record: &SessionRecord, repo: &Path) -> bool {
        let Ok(config) = self.repo_config(repo).await else {
            return true;
        };
        select_preset(repo, &config, Some(&record.preset))
            .map_or(true, |preset| push_asks(preset.mode))
    }

    async fn ask_push(
        &self,
        id: &SessionId,
        broker: GuardId,
        branch: &str,
        leased: bool,
    ) -> Result<(), String> {
        let hit = GuardHit {
            kind: GuardKind::Push,
            target: branch.into(),
        };
        let (waiter, answer) = oneshot::channel();
        {
            let mut state = self.lock();
            let live = state.sessions.get_mut(id).ok_or("the Session has ended")?;
            if live.record.guard_allowances.contains(&hit) {
                return Ok(());
            }
            let tool = match leased {
                true => "git push --force-with-lease",
                false => "git push",
            };
            live.prompts.push(PendingGuard {
                id: broker,
                tool: tool.into(),
                hit,
                brokered: Some(waiter),
            });
            live.status
                .observe(Observation::GuardPrompted, Instant::now());
            state.changed(id);
        }
        match answer.await {
            Ok(GuardAnswer::Proceed) => Ok(()),
            _ => Err(DENIED.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMIT: &str = "c0ffee0000000000000000000000000000000000";

    fn update(src: &str, dst: &str) -> RefUpdate {
        RefUpdate {
            src: src.into(),
            commit: (!src.is_empty()).then(|| COMMIT.into()),
            dst: dst.into(),
            force: false,
            lease: None,
        }
    }

    fn check(updates: &[RefUpdate]) -> Result<AllowedPush, String> {
        allowed_push(updates, "orch/work", "main")
    }

    #[test]
    fn the_sessions_own_branch_is_pushed_from_any_source_commit() {
        let pushed = check(&[update("HEAD~1", "refs/heads/orch/work")]);
        assert_eq!(
            pushed,
            Ok(AllowedPush {
                commit: COMMIT.into(),
                lease: None,
            })
        );
    }

    #[test]
    fn a_lease_force_goes_through_but_a_plain_force_is_refused() {
        let mut leased = update("HEAD", "refs/heads/orch/work");
        leased.force = true;
        leased.lease = Some(String::new());
        assert!(check(&[leased.clone()]).is_ok());

        leased.lease = None;
        let refusal = check(&[leased]).unwrap_err();
        assert!(refusal.contains("--force-with-lease"), "{refusal}");
    }

    #[test]
    fn the_base_branch_tags_other_branches_and_deletions_are_refused() {
        for (update, says) in [
            (update("HEAD", "refs/heads/main"), "Base branch main"),
            (update("v1", "refs/tags/v1"), "tags"),
            (update("HEAD", "refs/heads/other"), "refs/heads/other"),
            (update("", "refs/heads/orch/work"), "delete"),
        ] {
            let refusal = check(&[update]).unwrap_err();
            assert!(refusal.contains(says), "{refusal}");
        }
    }

    #[test]
    fn a_commit_that_is_not_a_full_object_id_is_refused() {
        for commit in [
            "--repo=elsewhere",
            &format!("+{COMMIT}"),
            "c0ffee",
            &COMMIT.to_uppercase(),
            "",
        ] {
            let mut forged = update("HEAD", "refs/heads/orch/work");
            forged.commit = Some(commit.into());
            assert!(check(&[forged]).is_err(), "{commit:?} went through");
        }
        let mut sha256 = update("HEAD", "refs/heads/orch/work");
        sha256.commit = Some("ab".repeat(32));
        assert!(check(&[sha256]).is_ok());
    }

    #[test]
    fn a_lease_that_is_not_absent_or_a_full_object_id_is_refused() {
        for lease in ["--repo=elsewhere", "c0ffee", "HEAD"] {
            let mut forged = update("HEAD", "refs/heads/orch/work");
            forged.force = true;
            forged.lease = Some(lease.into());
            assert!(check(&[forged]).is_err(), "{lease:?} went through");
        }
        let mut leased = update("HEAD", "refs/heads/orch/work");
        leased.force = true;
        leased.lease = Some(COMMIT.into());
        assert!(check(&[leased]).is_ok());
    }

    #[test]
    fn a_batch_with_any_refused_ref_pushes_nothing() {
        let all = [
            update("refs/heads/orch/work", "refs/heads/orch/work"),
            update("refs/heads/other", "refs/heads/other"),
        ];
        assert!(check(&all).is_err());
    }
}
