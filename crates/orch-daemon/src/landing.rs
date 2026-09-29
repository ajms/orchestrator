use std::path::{Path, PathBuf};
use std::sync::Arc;

use orch_core::{Phase, PhaseEvent, PrState, Retarget, SessionId, retarget};
use orch_git::{LandingError, Script};
use orch_protocol::{Landing, Reply, RequestError};
use orch_store::SessionRecord;

use crate::cleanup::clean_up_ended;
use crate::lifecycle::{Busy, HOLDER_EXIT_WAIT, refused, session_env, with_git};
use crate::pr::retarget_pr;
use crate::state::{Daemon, Live, gate_message, session_worktree};

pub(crate) struct Exclusive {
    daemon: Arc<Daemon>,
    id: SessionId,
}

impl Drop for Exclusive {
    fn drop(&mut self) {
        let mut state = self.daemon.lock();
        let Some(live) = state.sessions.get_mut(&self.id) else {
            return;
        };
        live.exclusive = false;
        if live.deliver_prompt() {
            state.changed(&self.id);
        }
    }
}

struct StackedPr {
    session: SessionId,
    number: u64,
    base: String,
}

fn conflict_prompt(base: &str, paths: &[String]) -> String {
    format!(
        "Landing this Branch onto {base} conflicted in: {}. Please rebase this Branch onto {base} (git rebase {base}), resolve the conflicts and commit the result.",
        paths.join(", ")
    )
}

fn stacked_prompt(base: &str, landed: &str) -> String {
    format!(
        "The Branch this Session was stacked on ({landed}) has been Landed into {base}. Please rebase this Branch onto {base}, dropping the commits that were already Landed, resolve any conflicts and commit the result."
    )
}

fn skipped_teardown(reason: impl std::fmt::Display) -> String {
    format!("the Teardown script was skipped: {reason}")
}

impl Daemon {
    pub(crate) fn claim(
        self: &Arc<Self>,
        id: &SessionId,
        gate: impl FnOnce(&Live) -> Result<(), String>,
    ) -> Result<(Exclusive, SessionRecord, PathBuf), RequestError> {
        let mut state = self.lock();
        let live = state
            .sessions
            .get_mut(id)
            .ok_or(RequestError::UnknownSession)?;
        if live.exclusive {
            return Err(refused(
                "another Landing, Discard or Preset change of this Session is in progress",
            ));
        }
        gate(live).map_err(refused)?;
        live.exclusive = true;
        let claimed = Exclusive {
            daemon: self.clone(),
            id: id.clone(),
        };
        Ok((claimed, live.record.clone(), live.repo.clone()))
    }

    pub(crate) async fn land(
        self: &Arc<Self>,
        id: &SessionId,
        landing: Landing,
        skip_teardown: bool,
    ) -> Result<Reply, RequestError> {
        let _busy = Busy::new(self);
        let (_claim, record, repo) = self.claim(id, |live| {
            if live.status.flags().base_missing {
                return Err(format!(
                    "the Base branch {} is gone; retarget the Session first",
                    live.record.base
                ));
            }
            live.status
                .check_landing()
                .map_err(|refusal| gate_message("Landing", refusal))
        })?;
        match landing {
            Landing::Squash { message } => {
                if !skip_teardown {
                    self.check_teardown(&repo).await?;
                }
                self.land_squash(id, record, repo, message).await
            }
            Landing::Pr { title, body } => self.open_pr(id, record, repo, title, body).await,
        }
    }

    async fn land_squash(
        self: &Arc<Self>,
        id: &SessionId,
        record: SessionRecord,
        repo: PathBuf,
        message: String,
    ) -> Result<Reply, RequestError> {
        let worktree = session_worktree(&record);
        let landed = with_git(repo, move |git| git.land_squash(&worktree, &message)).await?;
        match landed {
            Ok(landed) => {
                let warning = match self.end_session(id, PhaseEvent::Landed).await {
                    Ok(problem) => problem,
                    Err(err) => Some(format!("cleaning up after the Landing failed: {err}")),
                };
                Ok(Reply::Landed {
                    commit: landed.commit,
                    warning,
                })
            }
            Err(LandingError::Conflict { paths }) => {
                let prompt = conflict_prompt(&record.base, &paths);
                let _ = self.update(id, |live| {
                    live.status.flag_needs_rebase();
                    live.hand_back(prompt);
                    Ok(())
                });
                Err(RequestError::Conflict { paths })
            }
            Err(err) => Err(refused(err)),
        }
    }

    pub(crate) async fn end_session(
        self: &Arc<Self>,
        id: &SessionId,
        event: PhaseEvent,
    ) -> Result<Option<String>, RequestError> {
        let mut holder = None;
        let mut setting_up = false;
        self.update(id, |live| {
            setting_up = live.status.phase() == Phase::SettingUp;
            live.transition(event)?;
            holder = live.replace_holder(None).1;
            live.record.queued_prompt = None;
            Ok(())
        })?;
        if let Some(link) = holder {
            self.release_holder(link, HOLDER_EXIT_WAIT).await;
        }
        if setting_up {
            self.stop_setup(id).await;
        }
        let (record, repo) = self.snapshot(id).ok_or(RequestError::UnknownSession)?;
        let mut problems = Vec::new();
        let teardown = match self.teardown_script(&repo).await {
            Ok(script) => script.map(|command| Script {
                command,
                env: session_env(&record),
            }),
            Err(problem) => {
                problems.push(problem);
                None
            }
        };
        let worktree = session_worktree(&record);
        let dir = self.session_dir(id);
        let removed = with_git(repo.clone(), move |git| {
            clean_up_ended(git, &worktree, teardown.as_ref(), &dir)
        })
        .await?;
        match removed {
            Ok(Some(outcome)) if !outcome.success => problems.push(format!(
                "the Teardown script failed:\n{}",
                outcome.output.trim_end()
            )),
            Ok(_) => {}
            Err(err) => problems.push(format!("removing the Worktree and Branch failed: {err}")),
        }
        if let Err(problem) = self.free_port_block(id).await {
            problems.push(problem);
        }
        let problem = (!problems.is_empty()).then(|| problems.join("\n"));
        self.update(id, |live| {
            live.record.port_block = None;
            live.last_error.clone_from(&problem);
            Ok(())
        })?;
        if matches!(event, PhaseEvent::Landed | PhaseEvent::PrMerged) {
            let stacked = self.retarget_stacked(&record);
            self.retarget_prs(&repo, stacked).await;
        }
        self.forget_session(id).await;
        Ok(problem)
    }

    pub(crate) async fn teardown_script(&self, repo: &Path) -> Result<Option<String>, String> {
        let config = self.repo_config(repo).await.map_err(skipped_teardown)?;
        config
            .teardown_script()
            .map(|script| script.map(String::from))
            .map_err(skipped_teardown)
    }

    fn retarget_stacked(&self, landed: &SessionRecord) -> Vec<StackedPr> {
        let mut state = self.lock();
        let bases: Vec<(SessionId, String)> = state
            .sessions
            .values()
            .filter(|live| !live.status.phase().is_terminal() && live.record.id != landed.id)
            .map(|live| (live.record.id.clone(), live.record.base.clone()))
            .collect();
        let moves = retarget(
            bases.iter().map(|(id, base)| (id, base.as_str())),
            &landed.branch,
            &landed.base,
        );
        let mut prs = Vec::new();
        for Retarget { session, base } in moves {
            let Some(live) = state.sessions.get_mut(&session) else {
                continue;
            };
            live.record.base.clone_from(&base);
            live.status.flag_needs_rebase();
            live.hand_back(stacked_prompt(&base, &landed.branch));
            if let Some(pr) = &live.status.flags().pr
                && pr.state == PrState::Open
            {
                prs.push(StackedPr {
                    session: session.clone(),
                    number: pr.number,
                    base,
                });
            }
            state.changed(&session);
        }
        prs
    }

    async fn retarget_prs(&self, repo: &Path, stacked: Vec<StackedPr>) {
        for StackedPr {
            session,
            number,
            base,
        } in stacked
        {
            if let Err(err) = retarget_pr(repo, number, &base).await {
                let _ = self.update(&session, |live| {
                    live.last_error = Some(format!(
                        "retargeting PR #{number} onto {base} failed: {err}"
                    ));
                    Ok(())
                });
            }
        }
    }

    pub(crate) fn request_recheck(self: &Arc<Self>, id: &SessionId) {
        let mut state = self.lock();
        let Some(live) = state.sessions.get_mut(id) else {
            return;
        };
        if live.recheck.running {
            live.recheck.again = true;
            return;
        }
        live.recheck.running = true;
        tokio::spawn(self.clone().recheck_rebase(id.clone()));
    }

    async fn recheck_rebase(self: Arc<Self>, id: SessionId) {
        loop {
            if let Some((record, repo)) = self.snapshot(&id) {
                let worktree = session_worktree(&record);
                let contains = with_git(repo, move |git| git.contains_base_tip(&worktree)).await;
                if let Ok(Ok(contains)) = contains {
                    let _ = self.update(&id, |live| {
                        live.status.rebase_checked(contains);
                        Ok(())
                    });
                }
            }
            let mut state = self.lock();
            let Some(live) = state.sessions.get_mut(&id) else {
                return;
            };
            if !std::mem::take(&mut live.recheck.again) {
                live.recheck.running = false;
                return;
            }
        }
    }
}
