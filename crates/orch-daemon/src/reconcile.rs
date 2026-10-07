use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use orch_config::{GlobalConfig, RepoConfig};
use orch_core::{Phase, PhaseEvent, SessionId};
use orch_git::{InUse, Leftover, Script, SessionName, slugify};
use orch_holder::{Hello, HolderClient, ToHolder, holders_dir};
use orch_protocol::{
    CommitView, Finding, Fix, LeftoverView, Problem, ReconcileReport, Repair, Reply, RepoReport,
    RequestError,
};
use orch_store::{NewSession, PortBlock, RepoRoot, SessionRecord};

use crate::agents::adapter_by_name;
use crate::cleanup::{CleanupMarker, remove_session_dir};
use crate::lifecycle::{
    Busy, HOLDER_EXIT_WAIT, new_session_id, refused, select_preset, session_env, with_git,
};
use crate::pr::{open_pr_of, retarget_pr};
use crate::state::{CommentCursor, Daemon, Live, session_worktree};

const HELLO_WAIT: Duration = Duration::from_secs(2);
const FALLBACK_PRESET: &str = "inherit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pass {
    Cheap,
    Full,
}

struct HolderWorktree {
    root: RepoRoot,
    name: SessionName,
    worktree: PathBuf,
}

struct NewRecord {
    id: SessionId,
    root: RepoRoot,
    name: SessionName,
    worktree: PathBuf,
    base: String,
    phase: Phase,
    preset: String,
    agent: String,
    port_block: Option<PortBlock>,
}

struct SessionCheck {
    id: SessionId,
    base: String,
    worktree_present: bool,
    branch_present: bool,
    base_present: bool,
}

fn leftover_view(leftover: Leftover) -> LeftoverView {
    match leftover {
        Leftover::Worktree(path) => LeftoverView::Worktree { path },
        Leftover::Branch(branch) => LeftoverView::Branch { branch },
    }
}

fn leftover_of(view: LeftoverView) -> Leftover {
    match view {
        LeftoverView::Worktree { path } => Leftover::Worktree(path),
        LeftoverView::Branch { branch } => Leftover::Branch(branch),
    }
}

fn slug_of(path: &Path) -> Option<String> {
    path.file_name()?.to_str().map(String::from)
}

fn locate_holder_worktree(cwd: &Path) -> Option<HolderWorktree> {
    let root = RepoRoot::resolve(cwd).ok()?;
    let git = orch_git::Repo::open(root.path()).ok()?;
    let slug = slug_of(cwd)?;
    let worktree = git.worktree_path(&slug);
    if worktree.canonicalize().ok()? != cwd.canonicalize().ok()? {
        return None;
    }
    let branch = git.worktree_branch(&worktree).ok()??;
    Some(HolderWorktree {
        root,
        name: SessionName { slug, branch },
        worktree,
    })
}

async fn holder_hello(socket: &Path) -> Option<Hello> {
    let hello = async {
        let mut client = HolderClient::connect(socket).await?;
        client.attach().await
    };
    tokio::time::timeout(HELLO_WAIT, hello).await.ok()?.ok()
}

fn repo_report<'a>(report: &'a mut ReconcileReport, repo: &Path) -> Option<&'a mut RepoReport> {
    report.repos.iter_mut().find(|found| found.repo == repo)
}

impl Daemon {
    pub(crate) async fn reconcile_loop(self: Arc<Self>) {
        self.reconcile(Pass::Full).await;
        loop {
            tokio::time::sleep(self.config.reconcile_interval).await;
            self.reconcile(Pass::Cheap).await;
        }
    }

    pub(crate) async fn reconcile_now(self: &Arc<Self>) -> Result<Reply, RequestError> {
        let report = self.reconcile(Pass::Full).await;
        let daemon = self.clone();
        tokio::spawn(async move { daemon.poll_waiting_prs().await });
        Ok(Reply::Reconciled {
            report: Box::new(report),
        })
    }

    pub(crate) async fn reconcile(self: &Arc<Self>, pass: Pass) -> ReconcileReport {
        let ticket = {
            let mut state = self.lock();
            state.passes.requested += 1;
            match pass {
                Pass::Full => {
                    state.passes.full_requested += 1;
                    state.passes.full_requested
                }
                Pass::Cheap => state.passes.requested,
            }
        };
        let _running = self.reconciling.lock().await;
        let (covers, covers_full, run) = {
            let state = self.lock();
            let done = match pass {
                Pass::Full => state.passes.full_completed,
                Pass::Cheap => state.passes.completed,
            };
            if done >= ticket {
                return state.report.clone().unwrap_or_default();
            }
            let full_pending = state.passes.full_requested > state.passes.full_completed;
            let run = match (pass, full_pending) {
                (Pass::Cheap, false) => Pass::Cheap,
                _ => Pass::Full,
            };
            (state.passes.requested, state.passes.full_requested, run)
        };
        let _busy = Busy::new(self);
        let report = self.run_pass(run).await;
        let mut state = self.lock();
        state.passes.completed = covers;
        if run == Pass::Full {
            state.passes.full_completed = covers_full;
        }
        state.publish_report(&report);
        report
    }

    async fn run_pass(self: &Arc<Self>, pass: Pass) -> ReconcileReport {
        let mut report = ReconcileReport::default();
        self.adopt_unknown_holders(&mut report).await;
        self.suspend_sessions_without_holder(&mut report).await;
        self.check_repos(pass, &mut report).await;
        self.sync_port_blocks(&mut report).await;
        if pass == Pass::Cheap {
            self.carry_forward(&mut report);
        }
        report
    }

    fn carry_forward(&self, report: &mut ReconcileReport) {
        let Some(previous) = self.lock().report.clone() else {
            return;
        };
        for earlier in previous.repos {
            let Some(current) = repo_report(report, &earlier.repo) else {
                continue;
            };
            if current.missing {
                continue;
            }
            current
                .findings
                .extend(earlier.findings.into_iter().filter(|finding| {
                    matches!(
                        finding.problem,
                        Problem::Leftover { .. } | Problem::CleanupFailed { .. }
                    )
                }));
        }
    }

    async fn global_config(&self) -> Result<GlobalConfig, RequestError> {
        let loader = self.config.loader.clone();
        tokio::task::spawn_blocking(move || loader.global())
            .await
            .map_err(refused)?
            .map_err(refused)
    }

    pub(crate) async fn branch_prefix(&self) -> String {
        self.global_config().await.map_or_else(
            |_| orch_git::DEFAULT_BRANCH_PREFIX.into(),
            |global| global.branch_prefix,
        )
    }

    pub(crate) async fn default_base(
        &self,
        repo: &Path,
        config: &RepoConfig,
    ) -> Result<String, RequestError> {
        let configured = config.base_branch().map(String::from);
        with_git(repo.to_path_buf(), move |git| {
            git.default_base(configured.as_deref())
        })
        .await?
        .map_err(refused)
    }

    pub(crate) async fn registered_repo(
        &self,
        repo: &Path,
    ) -> Result<orch_store::Repo, RequestError> {
        let known = repo.to_path_buf();
        self.store
            .call(move |store| store.repo_by_path(&known))
            .await?
            .map_err(refused)?
            .ok_or_else(|| refused("no such Repo"))
    }

    async fn create_record(
        self: &Arc<Self>,
        new: NewRecord,
    ) -> Result<SessionRecord, RequestError> {
        let ports = self.global_config().await?.ports;
        let repo = new.root.path().to_path_buf();
        let record = self
            .store
            .call(move |store| {
                let registered = store.register_repo(&new.root)?;
                let created = store.create_session(NewSession {
                    id: new.id,
                    repo: registered.id,
                    slug: new.name.slug,
                    branch: new.name.branch,
                    base: new.base,
                    worktree: new.worktree,
                    phase: new.phase,
                    preset: new.preset,
                    agent: new.agent,
                })?;
                match new.port_block {
                    Some(block) => drop(store.hold_port_block(&created.id, block)?),
                    None => drop(store.allocate_port_block(&created.id, &ports)?),
                }
                store
                    .session(&created.id)
                    .map(|found| found.unwrap_or(created))
            })
            .await?
            .map_err(refused)?;
        let adapter = adapter_by_name(&record.agent);
        self.lock().insert(Live::new(record.clone(), repo, adapter));
        Ok(record)
    }

    async fn unknown_holders(&self) -> Vec<SessionId> {
        let dir = holders_dir(&self.config.runtime_dir);
        let listed = tokio::task::spawn_blocking(move || {
            std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|entry| {
                    let path = entry.path();
                    (path.extension()? == "sock")
                        .then(|| SessionId::parse(path.file_stem()?.to_str()?).ok())
                        .flatten()
                })
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        let state = self.lock();
        listed
            .into_iter()
            .filter(|id| !state.sessions.contains_key(id))
            .collect()
    }

    async fn adopt_unknown_holders(self: &Arc<Self>, report: &mut ReconcileReport) {
        for id in self.unknown_holders().await {
            let Some(hello) = holder_hello(&self.holder_socket(&id)).await else {
                continue;
            };
            if self.adopt_holder(&id, &hello).await.is_some() {
                report.repaired.push(Repair::AdoptedHolder { session: id });
                continue;
            }
            report.unknown_holders.push(Finding {
                problem: Problem::UnknownHolder {
                    session: id.clone(),
                    holder_pid: hello.holder_pid,
                    cwd: hello.cwd.clone(),
                },
                fixes: vec![Fix::ShutdownUnknownHolder { session: id }],
            });
        }
    }

    async fn adopt_holder(self: &Arc<Self>, id: &SessionId, hello: &Hello) -> Option<()> {
        let known = id.clone();
        let recorded = self.store.call(move |store| store.session(&known)).await;
        if !matches!(recorded, Ok(Ok(None))) {
            return None;
        }
        let cwd = hello.cwd.clone()?;
        let found = tokio::task::spawn_blocking(move || locate_holder_worktree(&cwd))
            .await
            .ok()??;
        let repo = found.root.path().to_path_buf();
        let branch = found.name.branch.clone();
        {
            let _guard = self.repo_guard(&repo).await;
            let config = self.repo_config(&repo).await.ok()?;
            let base = match &hello.base {
                Some(base) => base.clone(),
                None => self.default_base(&repo, &config).await.ok()?,
            };
            let preset = select_preset(&repo, &config, None)
                .map_or_else(|_| FALLBACK_PRESET.into(), |preset| preset.name);
            self.create_record(NewRecord {
                id: id.clone(),
                root: found.root,
                name: found.name,
                worktree: found.worktree,
                base,
                phase: Phase::Active,
                preset,
                agent: hello
                    .agent_name
                    .clone()
                    .unwrap_or_else(|| config.default_agent().into()),
                port_block: hello.port_block,
            })
            .await
            .ok()?;
        }
        let _ = self.update(id, |live| {
            live.status.set_recovered(true);
            Ok(())
        });
        self.adopt_or_suspend(id).await;
        if let Some(number) = open_pr_of(&repo, &branch).await {
            let _ = self.update(id, |live| {
                live.comments = CommentCursor::opened();
                live.transition(PhaseEvent::PrOpened { number })
            });
        }
        Some(())
    }

    async fn suspend_sessions_without_holder(self: &Arc<Self>, report: &mut ReconcileReport) {
        let without_holder: Vec<SessionId> = self
            .lock()
            .sessions
            .values()
            .filter(|live| {
                live.status.phase().is_live()
                    && !live.has_holder()
                    && !live.launching
                    && !live.exclusive
                    && live.last_error.is_none()
            })
            .map(|live| live.record.id.clone())
            .collect();
        for id in without_holder {
            if self.adopt_or_suspend(&id).await {
                report
                    .repaired
                    .push(Repair::SuspendedSession { session: id });
            }
        }
    }

    async fn check_repos(self: &Arc<Self>, pass: Pass, report: &mut ReconcileReport) {
        let listed = self
            .store
            .call(|store| Ok::<_, orch_store::StoreError>((store.repos()?, store.sessions()?)))
            .await;
        let Ok(Ok((repos, records))) = listed else {
            return;
        };
        let prefix = self.branch_prefix().await;
        for repo in repos {
            let mut findings = Vec::new();
            self.mark_repo_missing(&repo.path, repo.missing);
            if repo.missing {
                findings.push(Finding {
                    problem: Problem::RepoMissing,
                    fixes: vec![Fix::ForgetRepo {
                        repo: repo.path.clone(),
                    }],
                });
            } else {
                if pass == Pass::Full {
                    let _guard = self.repo_guard(&repo.path).await;
                    let ended = records
                        .iter()
                        .filter(|record| record.repo == repo.id && record.phase.is_terminal());
                    for record in ended {
                        self.finish_cleanup(record, &repo.path, &records, report, &mut findings)
                            .await;
                    }
                }
                self.check_sessions(&repo.path, &mut findings).await;
                if pass == Pass::Full {
                    self.list_leftovers(&repo.path, &prefix, &mut findings)
                        .await;
                }
            }
            report.repos.push(RepoReport {
                repo: repo.path,
                missing: repo.missing,
                findings,
            });
        }
    }

    fn mark_repo_missing(&self, repo: &Path, missing: bool) {
        let mut state = self.lock();
        let flipped: Vec<SessionId> = state
            .sessions
            .values_mut()
            .filter(|live| live.repo == repo && live.repo_missing != missing)
            .map(|live| {
                live.repo_missing = missing;
                live.record.id.clone()
            })
            .collect();
        for id in flipped {
            state.changed(&id);
        }
    }

    async fn finish_cleanup(
        &self,
        record: &SessionRecord,
        repo: &Path,
        records: &[SessionRecord],
        report: &mut ReconcileReport,
        findings: &mut Vec<Finding>,
    ) {
        if self.lock().sessions.contains_key(&record.id) {
            return;
        }
        let dir = self.session_dir(&record.id);
        let marked = dir.clone();
        let Some(marker) = tokio::task::spawn_blocking(move || CleanupMarker::read(&marked))
            .await
            .ok()
            .flatten()
        else {
            return;
        };
        let superseded = records.iter().any(|other| {
            other.id != record.id
                && (other.worktree == record.worktree || other.branch == record.branch)
                && other.created_at > record.created_at
        });
        if superseded {
            let _ = tokio::task::spawn_blocking(move || remove_session_dir(&dir)).await;
            return;
        }
        let in_use = {
            let state = self.lock();
            let mut in_use = InUse::default();
            for live in state.sessions.values() {
                in_use.worktree |= live.record.worktree == record.worktree;
                in_use.branch |= live.record.branch == record.branch;
            }
            in_use
        };
        let teardown = match marker.teardown_done {
            true => None,
            false => self
                .teardown_script(repo)
                .await
                .ok()
                .flatten()
                .map(|command| Script {
                    command,
                    env: session_env(record),
                }),
        };
        let worktree = session_worktree(record);
        let checkpoint = marker.checkpoint();
        let cleaned = with_git(repo.to_path_buf(), move |git| {
            let cleaned =
                git.finish_session_cleanup(&worktree, &checkpoint, in_use, teardown.as_ref())?;
            Ok::<_, orch_git::Error>((cleaned, remove_session_dir(&dir)))
        })
        .await;
        let session = record.id.clone();
        match cleaned {
            Ok(Ok((cleaned, Ok(())))) if !cleaned.kept_worktree && !cleaned.kept_branch => {
                report.repaired.push(Repair::FinishedCleanup { session });
            }
            Ok(Ok((_, Ok(())))) | Err(_) => {}
            Ok(Ok((_, Err(message)))) => findings.push(Finding {
                problem: Problem::CleanupFailed { session, message },
                fixes: Vec::new(),
            }),
            Ok(Err(err)) => findings.push(Finding {
                problem: Problem::CleanupFailed {
                    session,
                    message: err.to_string(),
                },
                fixes: Vec::new(),
            }),
        }
    }

    async fn check_sessions(&self, repo: &Path, findings: &mut Vec<Finding>) {
        let records: Vec<SessionRecord> = self
            .lock()
            .sessions
            .values()
            .filter(|live| {
                live.repo == repo && !live.status.phase().is_terminal() && !live.exclusive
            })
            .map(|live| live.record.clone())
            .collect();
        if records.is_empty() {
            return;
        }
        let configured = self
            .repo_config(repo)
            .await
            .ok()
            .and_then(|config| config.base_branch().map(String::from));
        let checked = with_git(repo.to_path_buf(), move |git| {
            let default = git
                .default_base(configured.as_deref())
                .ok()
                .filter(|base| git.branch_exists(base));
            let checks: Vec<SessionCheck> = records
                .iter()
                .map(|record| SessionCheck {
                    id: record.id.clone(),
                    base: record.base.clone(),
                    worktree_present: record.worktree.is_dir(),
                    branch_present: git.branch_exists(&record.branch),
                    base_present: git.branch_exists(&record.base),
                })
                .collect();
            Ok::<_, orch_git::Error>((default, checks))
        })
        .await;
        let Ok(Ok((default, checks))) = checked else {
            return;
        };
        for check in checks {
            self.set_presence_flags(&check);
            let session = check.id;
            if !check.worktree_present {
                let fix = match check.branch_present {
                    true => Fix::RecreateWorktree {
                        session: session.clone(),
                    },
                    false => Fix::DiscardRecord {
                        session: session.clone(),
                    },
                };
                findings.push(Finding {
                    problem: Problem::WorktreeMissing {
                        session: session.clone(),
                    },
                    fixes: vec![fix],
                });
            }
            if !check.base_present {
                let fixes = default
                    .iter()
                    .map(|base| Fix::Retarget {
                        session: session.clone(),
                        base: base.clone(),
                    })
                    .collect();
                findings.push(Finding {
                    problem: Problem::BaseMissing {
                        session,
                        base: check.base,
                    },
                    fixes,
                });
            }
        }
    }

    fn set_presence_flags(&self, check: &SessionCheck) {
        let mut state = self.lock();
        let Some(live) = state.sessions.get_mut(&check.id) else {
            return;
        };
        let flags = live.status.flags();
        if flags.worktree_missing == !check.worktree_present
            && flags.base_missing == !check.base_present
        {
            return;
        }
        live.status.set_worktree_missing(!check.worktree_present);
        live.status.set_base_missing(!check.base_present);
        state.changed(&check.id);
    }

    async fn leftovers(&self, repo: &Path, prefix: String) -> Result<Vec<Leftover>, RequestError> {
        let scanned = with_git(repo.to_path_buf(), move |git| git.leftovers(&prefix, &[]))
            .await?
            .map_err(refused)?;
        let state = self.lock();
        let known: Vec<&SessionRecord> = state
            .sessions
            .values()
            .filter(|live| live.repo == repo)
            .map(|live| &live.record)
            .collect();
        Ok(scanned
            .into_iter()
            .filter(|leftover| {
                !known.iter().any(|record| match leftover {
                    Leftover::Worktree(path) => *path == record.worktree,
                    Leftover::Branch(branch) => *branch == record.branch,
                })
            })
            .collect())
    }

    async fn list_leftovers(&self, repo: &Path, prefix: &str, findings: &mut Vec<Finding>) {
        let leftovers = {
            let _guard = self.repo_guard(repo).await;
            self.leftovers(repo, prefix.into()).await
        };
        let Ok(leftovers) = leftovers else {
            return;
        };
        for leftover in leftovers.into_iter().map(leftover_view) {
            findings.push(Finding {
                problem: Problem::Leftover {
                    leftover: leftover.clone(),
                },
                fixes: vec![
                    Fix::AdoptLeftover {
                        repo: repo.to_path_buf(),
                        leftover: leftover.clone(),
                    },
                    Fix::RemoveLeftover {
                        repo: repo.to_path_buf(),
                        leftover,
                    },
                ],
            });
        }
    }

    async fn sync_port_blocks(&self, report: &mut ReconcileReport) {
        let drifted: Vec<(SessionId, PortBlock)> = self
            .lock()
            .sessions
            .values()
            .filter_map(|live| {
                let held = live.holder_port_block()?;
                (live.record.port_block != Some(held)).then(|| (live.record.id.clone(), held))
            })
            .collect();
        for (id, block) in drifted {
            let session = id.clone();
            let held = self
                .store
                .call(move |store| store.hold_port_block(&session, block))
                .await;
            if matches!(held, Ok(Ok(_))) {
                let _ = self.update(&id, |live| {
                    live.record.port_block = Some(block);
                    Ok(())
                });
            }
        }
        let rebuilt = self
            .store
            .call(|store| store.rebuild_port_blocks().map(drop))
            .await;
        if let Ok(Err(err)) = rebuilt {
            eprintln!("orch daemon: rebuilding the Port blocks: {err}");
        }
        for (repo, finding) in self.port_block_clashes() {
            if let Some(repo) = repo_report(report, &repo) {
                repo.findings.push(finding);
            }
        }
    }

    fn port_block_clashes(&self) -> Vec<(PathBuf, Finding)> {
        let state = self.lock();
        let mut held: Vec<(&SessionRecord, &Path, PortBlock)> = state
            .sessions
            .values()
            .filter(|live| !live.status.phase().is_terminal())
            .filter_map(|live| Some((&live.record, live.repo.as_path(), live.record.port_block?)))
            .collect();
        held.sort_by(|a, b| (a.0.created_at, &a.0.id.0).cmp(&(b.0.created_at, &b.0.id.0)));
        let mut clashes = Vec::new();
        for (later, (record, repo, block)) in held.iter().enumerate() {
            for (earlier, _, other_block) in &held[..later] {
                if !block.overlaps(*other_block) {
                    continue;
                }
                clashes.push((
                    repo.to_path_buf(),
                    Finding {
                        problem: Problem::PortBlockClash {
                            session: record.id.clone(),
                            other: earlier.id.clone(),
                        },
                        fixes: vec![
                            Fix::ReassignPortBlock {
                                session: record.id.clone(),
                            },
                            Fix::ReassignPortBlock {
                                session: earlier.id.clone(),
                            },
                        ],
                    },
                ));
            }
        }
        clashes
    }

    pub(crate) async fn fix(self: &Arc<Self>, fix: Fix) -> Result<Reply, RequestError> {
        let _busy = Busy::new(self);
        let reply = match fix {
            Fix::RecreateWorktree { session } => self.recreate_worktree(&session).await,
            Fix::DiscardRecord { session } => self.discard_record(&session).await,
            Fix::Retarget { session, base } => self.retarget(&session, base).await,
            Fix::ForgetRepo { repo } => self.forget_repo(repo).await,
            Fix::AdoptLeftover { repo, leftover } => self.adopt_leftover(repo, leftover).await,
            Fix::RemoveLeftover { repo, leftover } => self.remove_leftover(repo, leftover).await,
            Fix::ShutdownUnknownHolder { session } => self.shutdown_unknown_holder(&session).await,
            Fix::ReassignPortBlock { session } => self.reassign_port_block(&session).await,
        }?;
        let daemon = self.clone();
        tokio::spawn(async move { daemon.reconcile(Pass::Full).await });
        Ok(reply)
    }

    async fn recreate_worktree(self: &Arc<Self>, id: &SessionId) -> Result<Reply, RequestError> {
        let (claim, record, repo) =
            self.claim(id, |live| match live.status.flags().worktree_missing {
                true => Ok(()),
                false => Err("the Worktree is not missing".into()),
            })?;
        let name = SessionName {
            slug: record.slug,
            branch: record.branch,
        };
        let path = {
            let _guard = self.repo_guard(&repo).await;
            with_git(repo, move |git| git.attach_worktree(&name))
                .await?
                .map_err(refused)?
        };
        let mut suspended = false;
        self.update(id, |live| {
            live.record.worktree = path;
            live.status.set_worktree_missing(false);
            suspended = live.status.phase() == Phase::Suspended;
            Ok(())
        })?;
        drop(claim);
        if suspended {
            self.resume(id).await?;
        }
        Ok(Reply::Done)
    }

    async fn discard_record(self: &Arc<Self>, id: &SessionId) -> Result<Reply, RequestError> {
        self.drop_session_record(id, |live| match live.status.flags().worktree_missing {
            true => Ok(()),
            false => Err("only a Session whose Worktree is missing ends as a record only".into()),
        })
        .await?;
        Ok(Reply::Done)
    }

    async fn retarget(
        self: &Arc<Self>,
        id: &SessionId,
        base: String,
    ) -> Result<Reply, RequestError> {
        let (_claim, _, repo) = self.claim(id, |live| match live.status.phase().is_terminal() {
            true => Err("the Session has ended".into()),
            false => Ok(()),
        })?;
        let wanted = base.clone();
        let exists = with_git(repo.clone(), move |git| {
            Ok::<_, orch_git::Error>(git.branch_exists(&wanted))
        })
        .await?
        .map_err(refused)?;
        if !exists {
            return Err(refused(format!("there is no Branch {base}")));
        }
        let mut pr = None;
        self.update(id, |live| {
            live.record.base.clone_from(&base);
            live.status.set_base_missing(false);
            live.status.flag_needs_rebase();
            pr = live.status.flags().pr.as_ref().map(|pr| pr.number);
            Ok(())
        })?;
        if let Some(number) = pr {
            retarget_pr(&repo, number, &base).await.map_err(refused)?;
        }
        Ok(Reply::Done)
    }

    async fn forget_repo(self: &Arc<Self>, repo: PathBuf) -> Result<Reply, RequestError> {
        let registered = self.registered_repo(&repo).await?;
        let _guard = self.repo_guard(&repo).await;
        let (sessions, running) = {
            let state = self.lock();
            let mine = || state.sessions.values().filter(|live| live.repo == repo);
            let sessions: Vec<SessionId> = mine().map(|live| live.record.id.clone()).collect();
            (sessions, mine().any(Live::is_running))
        };
        if !registered.missing && !sessions.is_empty() {
            return Err(refused(
                "the Repo still exists and has Sessions; Discard them first",
            ));
        }
        if running {
            return Err(refused(
                "Agents of the Repo are still running; quit them before forgetting it",
            ));
        }
        for id in &sessions {
            self.drop_session_record(id, |_| Ok(())).await?;
        }
        self.store
            .call(move |store| store.forget_repo(registered.id))
            .await?
            .map_err(refused)?;
        Ok(Reply::Done)
    }

    async fn drop_session_record(
        self: &Arc<Self>,
        id: &SessionId,
        gate: impl FnOnce(&Live) -> Result<(), String>,
    ) -> Result<(), RequestError> {
        let (claim, _, _) = self.claim(id, gate)?;
        let mut holder = None;
        self.update(id, |live| {
            live.transition(PhaseEvent::Discarded)?;
            holder = live.replace_holder(None).1;
            Ok(())
        })?;
        if let Some(link) = holder {
            self.release_holder(link, HOLDER_EXIT_WAIT).await;
        }
        let dir = self.session_dir(id);
        let _ = tokio::task::spawn_blocking(move || remove_session_dir(&dir)).await;
        let _ = self.free_port_block(id).await;
        self.update(id, |live| {
            live.record.port_block = None;
            Ok(())
        })?;
        drop(claim);
        self.forget_session(id).await;
        Ok(())
    }

    async fn current_leftover(
        &self,
        repo: &Path,
        leftover: LeftoverView,
    ) -> Result<Leftover, RequestError> {
        let leftover = leftover_of(leftover);
        let prefix = self.branch_prefix().await;
        match self.leftovers(repo, prefix).await?.contains(&leftover) {
            true => Ok(leftover),
            false => Err(refused("that is not a Leftover (any more)")),
        }
    }

    pub(crate) async fn leftover_preview(
        &self,
        repo: PathBuf,
        leftover: LeftoverView,
    ) -> Result<Reply, RequestError> {
        let leftover = self.current_leftover(&repo, leftover).await?;
        let config = self.repo_config(&repo).await?;
        let base = self.default_base(&repo, &config).await?;
        let preview = with_git(repo, move |git| git.leftover_preview(&leftover, &base))
            .await?
            .map_err(refused)?;
        Ok(Reply::DiscardPreview {
            uncommitted: preview.uncommitted,
            unlanded: preview
                .unlanded
                .into_iter()
                .map(|commit| CommitView {
                    id: commit.id,
                    subject: commit.subject,
                })
                .collect(),
        })
    }

    async fn adopt_leftover(
        self: &Arc<Self>,
        repo: PathBuf,
        leftover: LeftoverView,
    ) -> Result<Reply, RequestError> {
        let _guard = self.repo_guard(&repo).await;
        let leftover = self.current_leftover(&repo, leftover).await?;
        let config = self.repo_config(&repo).await?;
        let preset = select_preset(&repo, &config, None)?;
        let base = self.default_base(&repo, &config).await?;
        let prefix = self.branch_prefix().await;
        let (name, worktree) = with_git(repo.clone(), move |git| {
            let adopted = match leftover {
                Leftover::Worktree(path) => git
                    .worktree_branch(&path)?
                    .zip(slug_of(&path))
                    .map(|(branch, slug)| (SessionName { slug, branch }, path))
                    .ok_or("the directory is not a git Worktree on a Branch"),
                Leftover::Branch(branch) => {
                    let slug = slugify(branch.strip_prefix(&prefix).unwrap_or(&branch));
                    let name = SessionName { slug, branch };
                    let path = git.worktree_path(&name.slug);
                    if !path.exists() {
                        git.attach_worktree(&name)?;
                        Ok((name, path))
                    } else if git.worktree_branch(&path)?.as_ref() == Some(&name.branch) {
                        Ok((name, path))
                    } else {
                        Err("the Worktree directory for that Branch is taken")
                    }
                }
            };
            Ok::<_, orch_git::Error>(adopted)
        })
        .await?
        .map_err(refused)?
        .map_err(refused)?;
        let inside = repo.clone();
        let root = tokio::task::spawn_blocking(move || RepoRoot::resolve(&inside))
            .await
            .map_err(refused)?
            .map_err(refused)?;
        let record = self
            .create_record(NewRecord {
                id: new_session_id()?,
                root,
                name,
                worktree,
                base,
                phase: Phase::Suspended,
                preset: preset.name,
                agent: config.default_agent().into(),
                port_block: None,
            })
            .await?;
        Ok(Reply::Created { session: record.id })
    }

    async fn remove_leftover(
        &self,
        repo: PathBuf,
        leftover: LeftoverView,
    ) -> Result<Reply, RequestError> {
        let _guard = self.repo_guard(&repo).await;
        let leftover = self.current_leftover(&repo, leftover).await?;
        with_git(repo, move |git| git.remove_leftover(&leftover, None))
            .await?
            .map_err(refused)?;
        Ok(Reply::Done)
    }

    async fn shutdown_unknown_holder(&self, id: &SessionId) -> Result<Reply, RequestError> {
        if self.lock().sessions.contains_key(id) {
            return Err(refused("the Holder belongs to a Session"));
        }
        let shutdown = async {
            let mut client = HolderClient::connect(&self.holder_socket(id)).await?;
            client.attach().await?;
            client.send(&ToHolder::Shutdown).await
        };
        tokio::time::timeout(HELLO_WAIT, shutdown)
            .await
            .map_err(|_| refused("the Holder did not answer"))?
            .map_err(|err| refused(format!("cannot reach the Holder: {err}")))?;
        Ok(Reply::Done)
    }

    async fn reassign_port_block(self: &Arc<Self>, id: &SessionId) -> Result<Reply, RequestError> {
        let (claim, _, _) = self.claim(id, |live| match live.status.phase().is_terminal() {
            true => Err("the Session has ended".into()),
            false => Ok(()),
        })?;
        let ports = self.global_config().await?.ports;
        let session = id.clone();
        let block = self
            .store
            .call(move |store| {
                store.free_port_block(&session)?;
                store.allocate_port_block(&session, &ports)
            })
            .await?
            .map_err(refused)?;
        let mut replaced = None;
        let mut restart = false;
        self.update(id, |live| {
            live.record.port_block = Some(block);
            restart = live.status.phase().is_live() && live.has_holder();
            if restart {
                replaced = live.replace_holder(None).1;
                live.last_error = None;
                live.launching = true;
            }
            Ok(())
        })?;
        if let Some(link) = replaced {
            self.release_holder(link, HOLDER_EXIT_WAIT).await;
        }
        drop(claim);
        if restart {
            self.launch_agent(id, true).await;
        }
        Ok(Reply::Done)
    }
}
