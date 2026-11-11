//! Child Sessions: the `orch session` calls a Session's Agent makes through
//! its Holder socket, and the tree walks Landing, Discarding and muting need.
//!
//! The calls come from inside the Sandbox, so every id they name is checked:
//! a Session acts only on itself and its own descendants.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use orch_agent::GuardKind;
use orch_core::{
    AgentState, Attachment, ChildReport, ChildState, MAX_CHILD_IDS, MAX_CHILD_PROMPT,
    MAX_CHILD_SUMMARY, Phase, PlanOp, SessionId, SessionOp, checked_text,
};
use orch_holder::{BrokerReply, GuardId};
use orch_protocol::{CreateSession, Landing, Reply, RequestError};

use crate::lifecycle::{refused, select_preset};
use crate::state::{Daemon, Live};

/// Parent links are walked at most this far, so a corrupt cycle can't hang.
const MAX_WALK: usize = 64;

/// The Session's ancestors, nearest first.
pub(crate) fn ancestors(sessions: &HashMap<SessionId, Live>, id: &SessionId) -> Vec<SessionId> {
    let mut found = Vec::new();
    let mut at = sessions.get(id).and_then(|live| live.record.parent.clone());
    while let Some(parent) = at {
        if found.contains(&parent) || found.len() >= MAX_WALK {
            break;
        }
        at = sessions
            .get(&parent)
            .and_then(|live| live.record.parent.clone());
        found.push(parent);
    }
    found
}

/// How many ancestors the Session has: 0 for a top-level Session.
pub(crate) fn depth(sessions: &HashMap<SessionId, Live>, id: &SessionId) -> usize {
    ancestors(sessions, id).len()
}

pub(crate) fn is_descendant(
    sessions: &HashMap<SessionId, Live>,
    ancestor: &SessionId,
    id: &SessionId,
) -> bool {
    ancestors(sessions, id).contains(ancestor)
}

/// The Session's open descendants, deepest first, so ending them in order
/// never leaves a child without its parent.
pub(crate) fn open_descendants(
    sessions: &HashMap<SessionId, Live>,
    id: &SessionId,
) -> Vec<SessionId> {
    let mut found: Vec<(usize, SessionId)> = sessions
        .values()
        .filter(|live| !live.status.phase().is_terminal())
        .filter(|live| is_descendant(sessions, id, &live.record.id))
        .map(|live| (depth(sessions, &live.record.id), live.record.id.clone()))
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.as_str().cmp(b.1.as_str())));
    found.into_iter().map(|(_, id)| id).collect()
}

pub(crate) fn child_state(live: &Live) -> ChildState {
    if live.status.phase().is_terminal() {
        ChildState::Ended
    } else if live.record.land_failed.is_some() {
        ChildState::LandFailed
    } else if live.record.done_summary.is_some() {
        ChildState::Done
    } else if live.status.phase() == Phase::SetupFailed
        || matches!(
            live.status.agent_state(),
            Some(AgentState::Errored | AgentState::Exited)
        )
    {
        ChildState::Failed
    } else {
        ChildState::Working
    }
}

pub(crate) fn report(sessions: &HashMap<SessionId, Live>, id: &SessionId) -> ChildReport {
    let Some(live) = sessions.get(id) else {
        return ChildReport {
            id: id.as_str().into(),
            parent: None,
            branch: String::new(),
            state: ChildState::Ended,
            summary: None,
            problem: None,
        };
    };
    let state = child_state(live);
    ChildReport {
        id: id.as_str().into(),
        parent: live.record.parent.as_ref().map(|id| id.as_str().into()),
        branch: live.record.branch.clone(),
        state,
        summary: live.record.done_summary.clone(),
        problem: match state {
            ChildState::LandFailed => live.record.land_failed.clone(),
            ChildState::Failed => live.last_error.clone().or_else(|| {
                (live.status.phase() == Phase::SetupFailed).then(|| "Setup failed".into())
            }),
            _ => None,
        },
    }
}

/// `wait` returns once the child is settled; a Done child whose Agent is
/// still finishing its turn isn't, so the parent can Land it right away.
fn settled(sessions: &HashMap<SessionId, Live>, id: &SessionId) -> bool {
    let Some(live) = sessions.get(id) else {
        return true;
    };
    let state = child_state(live);
    let busy = matches!(
        live.status.agent_state(),
        Some(AgentState::Working | AgentState::Starting)
    );
    state.is_settled() && !(state == ChildState::Done && busy)
}

fn parse_id(id: &str) -> Result<SessionId, String> {
    SessionId::parse(id.trim()).map_err(|_| format!("{id:?} is not a Session id"))
}

fn queue_prompt(live: &mut Live, text: String) {
    let prompt = match live.record.queued_prompt.take() {
        Some(queued) => format!("{queued}\n\n{text}"),
        None => text,
    };
    live.hand_back(prompt);
}

fn render(reports: &[ChildReport]) -> String {
    reports
        .iter()
        .map(|report| {
            let mut line = format!("{} [{}]", report.id, report.state.name());
            if !report.branch.is_empty() {
                line.push_str(&format!(" {}", report.branch));
            }
            if let Some(problem) = &report.problem {
                line.push_str(&format!("\n  problem: {problem}"));
            }
            if let Some(summary) = &report.summary {
                line.push_str(&format!("\n  summary: {summary}"));
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

struct Waiting<'a> {
    daemon: &'a Daemon,
    id: SessionId,
    on: Vec<SessionId>,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        if let Some(live) = self.daemon.lock().sessions.get_mut(&self.id) {
            for child in &self.on {
                if let Some(at) = live.waiting_on.iter().position(|id| id == child) {
                    live.waiting_on.swap_remove(at);
                }
            }
        }
    }
}

impl Daemon {
    /// Boxed: creating a Session leads, through its Holder, back here, so the
    /// future's type must not depend on itself.
    pub(crate) fn session_op<'a>(
        self: &'a Arc<Self>,
        id: &'a SessionId,
        broker: GuardId,
        op: SessionOp,
    ) -> Pin<Box<dyn Future<Output = Result<BrokerReply, String>> + Send + 'a>> {
        Box::pin(self.serve_session_op(id, broker, op))
    }

    async fn serve_session_op(
        self: &Arc<Self>,
        id: &SessionId,
        broker: GuardId,
        op: SessionOp,
    ) -> Result<BrokerReply, String> {
        match op {
            SessionOp::Spawn {
                prompt,
                task,
                agent,
                preset,
            } => {
                self.spawn_child(id, broker, prompt, task, agent, preset)
                    .await
            }
            SessionOp::Done { summary } => self.child_done(id, summary),
            SessionOp::Wait { ids } => self.wait_children(id, ids).await,
            SessionOp::Status { ids } => self.child_status(id, ids),
            SessionOp::Land { id: child } => self.land_child(id, &child).await,
            SessionOp::Discard { id: child } => self.discard_child(id, &child).await,
        }
    }

    async fn spawn_child(
        self: &Arc<Self>,
        id: &SessionId,
        broker: GuardId,
        prompt: String,
        task: Option<orch_core::TaskId>,
        agent: Option<String>,
        preset: Option<String>,
    ) -> Result<BrokerReply, String> {
        let prompt = checked_text(&prompt, MAX_CHILD_PROMPT, "prompt")?;
        let (record, repo) = self.snapshot(id).ok_or("the Session has ended")?;
        if let Some(task) = task
            && record.plan.task(task).is_none()
        {
            return Err(format!("this Session's Plan has no Task {task}"));
        }
        if let Some(name) = preset.as_deref().filter(|name| *name != record.preset) {
            let config = self
                .repo_config(&repo)
                .await
                .map_err(|err| err.to_string())?;
            let ceiling =
                select_preset(&repo, &config, Some(&record.preset)).map_err(|e| e.to_string())?;
            let wanted = select_preset(&repo, &config, Some(name)).map_err(|e| e.to_string())?;
            if wanted.loosens_beyond(&ceiling) {
                let tool = format!("orch session spawn --preset {name}");
                self.ask(id, broker, GuardKind::ChildPreset, name, &tool)
                    .await
                    .map_err(|reason| {
                        format!(
                            "Preset {name} is looser than this Session's Preset {} and the user didn't approve it: {reason}",
                            record.preset
                        )
                    })?;
            }
        }
        let create = CreateSession {
            preset,
            agent,
            parent: Some(id.clone()),
            ..CreateSession::new(repo, prompt)
        };
        let child = match self.create_session(create).await {
            Ok(Reply::Created { session }) => session,
            Ok(other) => return Err(format!("unexpected reply: {other:?}")),
            Err(err) => return Err(err.to_string()),
        };
        let mut said = format!("started Child Session {}", child.as_str());
        if let Some(task) = task {
            let attached = self.update(id, |live| {
                live.record
                    .plan
                    .apply(PlanOp::Attach {
                        task,
                        attachment: Attachment::ChildSession(child.as_str().into()),
                    })
                    .map(drop)
            });
            match attached {
                Ok(()) => said.push_str(&format!(" on Task {task}")),
                Err(err) => said.push_str(&format!(" (attaching it to Task {task} failed: {err})")),
            }
        }
        let children = vec![report(&self.lock().sessions, &child)];
        Ok(BrokerReply::Session { said, children })
    }

    fn child_done(&self, id: &SessionId, summary: String) -> Result<BrokerReply, String> {
        let summary = checked_text(&summary, MAX_CHILD_SUMMARY, "summary")?;
        let mut state = self.lock();
        let live = state.sessions.get_mut(id).ok_or("the Session has ended")?;
        let parent = live
            .record
            .parent
            .clone()
            .ok_or("only a Child Session can be done; this Session has no parent")?;
        live.record.done_summary = Some(summary.clone());
        live.record.land_failed = None;
        state.changed(id);
        if let Some(parent_live) = state.sessions.get_mut(&parent)
            && !parent_live.waiting_on.contains(id)
        {
            queue_prompt(
                parent_live,
                format!(
                    "Child Session {} is done: {summary}\nLand it with `orch session land {}`, send it more work, or discard it.",
                    id.as_str(),
                    id.as_str()
                ),
            );
            state.changed(&parent);
        }
        let children = vec![report(&state.sessions, id)];
        Ok(BrokerReply::Session {
            said: "marked this Child Session Done; its parent decides whether to Land it".into(),
            children,
        })
    }

    /// The ids named, each of which must be the caller or a descendant;
    /// none names every descendant.
    fn reachable(&self, id: &SessionId, ids: &[String]) -> Result<Vec<SessionId>, String> {
        if ids.len() > MAX_CHILD_IDS {
            return Err(format!("name at most {MAX_CHILD_IDS} Sessions"));
        }
        let state = self.lock();
        if ids.is_empty() {
            let mut all = open_descendants(&state.sessions, id);
            all.reverse();
            return Ok(all);
        }
        ids.iter()
            .map(|text| {
                let child = parse_id(text)?;
                match &child == id || is_descendant(&state.sessions, id, &child) {
                    true => Ok(child),
                    false => Err(format!(
                        "{text} is not this Session or one of its Child Sessions"
                    )),
                }
            })
            .collect()
    }

    fn child_status(&self, id: &SessionId, ids: Vec<String>) -> Result<BrokerReply, String> {
        let ids = self.reachable(id, &ids)?;
        let state = self.lock();
        let children: Vec<_> = ids.iter().map(|id| report(&state.sessions, id)).collect();
        let said = match children.is_empty() {
            true => "this Session has no Child Sessions".into(),
            false => render(&children),
        };
        Ok(BrokerReply::Session { said, children })
    }

    async fn wait_children(&self, id: &SessionId, ids: Vec<String>) -> Result<BrokerReply, String> {
        if ids.is_empty() {
            return Err("name the Child Sessions to wait for".into());
        }
        let ids = self.reachable(id, &ids)?;
        if ids.contains(id) {
            return Err("a Session can't wait for itself".into());
        }
        let mut ticks = {
            let mut state = self.lock();
            let live = state.sessions.get_mut(id).ok_or("the Session has ended")?;
            live.waiting_on.extend(ids.iter().cloned());
            state.ticks.subscribe()
        };
        let _waiting = Waiting {
            daemon: self,
            id: id.clone(),
            on: ids.clone(),
        };
        loop {
            {
                let state = self.lock();
                if !state.sessions.contains_key(id) {
                    return Err("the Session has ended".into());
                }
                if ids.iter().all(|child| settled(&state.sessions, child)) {
                    let children: Vec<_> = ids
                        .iter()
                        .map(|child| report(&state.sessions, child))
                        .collect();
                    return Ok(BrokerReply::Session {
                        said: render(&children),
                        children,
                    });
                }
            }
            if ticks.changed().await.is_err() {
                return Err("the Orchestrator is shutting down".into());
            }
        }
    }

    /// A strict descendant: a Session can't Land or Discard itself this way.
    fn own_child(&self, id: &SessionId, child: &str) -> Result<SessionId, String> {
        let child = parse_id(child)?;
        match is_descendant(&self.lock().sessions, id, &child) {
            true => Ok(child),
            false => Err(format!(
                "{} is not one of this Session's Child Sessions",
                child.as_str()
            )),
        }
    }

    async fn land_child(
        self: &Arc<Self>,
        id: &SessionId,
        child: &str,
    ) -> Result<BrokerReply, String> {
        let child = self.own_child(id, child)?;
        let message = {
            let state = self.lock();
            let live = state
                .sessions
                .get(&child)
                .ok_or("the Child Session has ended")?;
            if live.record.done_summary.is_none() {
                return Err(format!(
                    "Child Session {} isn't Done yet; it Lands once it runs `orch session done`",
                    child.as_str()
                ));
            }
            let title = live
                .record
                .title
                .clone()
                .unwrap_or_else(|| live.record.slug.clone());
            match &live.record.done_summary {
                Some(summary) => format!("{title}\n\n{summary}"),
                None => title,
            }
        };
        match self.land(&child, Landing::Squash { message }, false).await {
            Ok(Reply::Landed { commit, warning }) => {
                let mut said = format!("Landed Child Session {} as {commit}", child.as_str());
                if let Some(warning) = warning {
                    said.push_str(&format!("\nwarning: {warning}"));
                }
                Ok(BrokerReply::Session {
                    said,
                    children: vec![report(&self.lock().sessions, &child)],
                })
            }
            Ok(other) => Err(format!("unexpected reply: {other:?}")),
            Err(RequestError::Conflict { paths }) => Err(format!(
                "Landing conflicts with this Session's Branch in: {}; the Child Session stays open, marked Land failed",
                paths.join(", ")
            )),
            Err(err) => Err(err.to_string()),
        }
    }

    async fn discard_child(
        self: &Arc<Self>,
        id: &SessionId,
        child: &str,
    ) -> Result<BrokerReply, String> {
        let child = self.own_child(id, child)?;
        let gone = {
            let state = self.lock();
            let mut gone = open_descendants(&state.sessions, &child);
            gone.push(child.clone());
            gone
        };
        self.discard(&child, false)
            .await
            .map_err(|err| err.to_string())?;
        let said = format!(
            "Discarded {}",
            gone.iter()
                .map(SessionId::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        );
        Ok(BrokerReply::Session {
            said,
            children: Vec::new(),
        })
    }

    /// A Session whose parent is gone becomes top-level, so no Child Session
    /// is left pointing at nothing (after a restart, or a parent whose PR was
    /// merged while it still had children).
    pub(crate) fn adopt_orphans(&self) {
        let mut state = self.lock();
        let orphans: Vec<SessionId> = state
            .sessions
            .values()
            .filter(|live| {
                live.record
                    .parent
                    .as_ref()
                    .is_some_and(|parent| match state.sessions.get(parent) {
                        Some(parent) => parent.status.phase().is_terminal(),
                        None => true,
                    })
            })
            .map(|live| live.record.id.clone())
            .collect();
        for id in orphans {
            if let Some(live) = state.sessions.get_mut(&id) {
                live.record.parent = None;
                live.last_error = Some(format!(
                    "its Parent Session ended, so it is a top-level Session now; its Base branch is {}",
                    live.record.base
                ));
            }
            state.changed(&id);
        }
    }

    /// Run after claiming: no child can be spawned while the Session is
    /// claimed, so this catches any spawned since the earlier check.
    pub(crate) fn refuse_open_children(&self, id: &SessionId) -> Result<(), RequestError> {
        let open = self.open_children_names(id);
        match open.is_empty() {
            true => Ok(()),
            false => Err(refused(format!(
                "this Session still has open Child Sessions ({}); Land or Discard them first",
                open.join(", ")
            ))),
        }
    }
    /// Muting a Session mutes its descendants too.
    pub(crate) fn set_muted(&self, id: &SessionId, muted: bool) -> Result<Reply, RequestError> {
        self.update(id, |live| {
            live.status.set_muted(muted);
            Ok(())
        })?;
        let descendants = open_descendants(&self.lock().sessions, id);
        for child in &descendants {
            let _ = self.update(child, |live| {
                live.status.set_muted(muted);
                Ok(())
            });
        }
        if muted {
            let state = self.lock();
            for session in std::iter::once(id).chain(&descendants) {
                state.dismiss(session);
            }
        }
        Ok(Reply::Done)
    }

    /// Names of the open descendants, for the Discard confirmation.
    pub(crate) fn session_name(&self, id: &SessionId) -> String {
        self.lock()
            .sessions
            .get(id)
            .map(|live| {
                live.record
                    .title
                    .clone()
                    .unwrap_or_else(|| live.record.slug.clone())
            })
            .unwrap_or_else(|| id.as_str().to_string())
    }

    pub(crate) fn open_children_names(&self, id: &SessionId) -> Vec<String> {
        let state = self.lock();
        open_descendants(&state.sessions, id)
            .iter()
            .filter_map(|child| state.sessions.get(child))
            .map(|live| {
                live.record
                    .title
                    .clone()
                    .unwrap_or_else(|| live.record.slug.clone())
            })
            .collect()
    }
}
