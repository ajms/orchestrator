use orch_core::SessionId;
use orch_protocol::{
    AgentStateView, CreateSession, Finding, Fix, Landing, LeftoverView, PhaseView, Problem,
    ReconcileReport, Repair, Reply, Request, RequestError,
};

use crate::common::*;

fn unknown_holder<'a>(findings: &'a [Finding], session: &SessionId) -> Option<&'a Finding> {
    findings.iter().find(
        |finding| matches!(&finding.problem, Problem::UnknownHolder { session: s, .. } if s == session),
    )
}

#[tokio::test]
async fn a_live_holder_without_a_record_is_adopted_and_flagged_recovered() {
    let env = Env::new();
    let repo = env.repo("app");
    git(&repo, &["branch", "develop"]);
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let mut create = CreateSession::new(&repo, "Lost records");
    create.base = Some("develop".into());
    let id = client.create(create).await;
    let before = client
        .until(&id, "running", |view| view.holder_pid.is_some())
        .await;

    daemon.kill();
    env.lose_state_db();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let adopted = client
        .until(&id, "adopted", |view| {
            view.flags.recovered && view.holder_pid.is_some()
        })
        .await;

    assert_eq!(adopted.phase, PhaseView::Active);
    assert_eq!(adopted.holder_pid, before.holder_pid);
    assert_eq!(adopted.repo, before.repo);
    assert_eq!(adopted.worktree, before.worktree);
    assert_eq!(adopted.branch, before.branch);
    assert_eq!(adopted.slug, before.slug);
    assert_eq!(adopted.base, "develop");
    assert_eq!(adopted.port_base, before.port_base);
    let mut pane = env.pane(&id, PANE).await;
    pane.type_line("print still here").await;
    pane.wait_for_text("still here").await;

    let other = running_session(&env, &mut client, "Another").await;
    assert_ne!(client.sessions[&other].port_base, before.port_base);
}

#[tokio::test]
async fn a_live_holder_outside_any_repo_is_reported_and_can_be_shut_down() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let unrelated = SessionId::parse("unrelated-holder").unwrap();
    let pid = env.hold(unrelated.as_str(), &env.path("home"));
    let mut client = env.client().await;

    let report = client.reconcile().await;
    let finding =
        unknown_holder(&report.unknown_holders, &unrelated).expect("unknown Holder listed");
    assert_eq!(
        finding.fixes,
        vec![Fix::ShutdownUnknownHolder {
            session: unrelated.clone()
        }]
    );
    assert!(client.session_list().await.is_empty());
    assert!(!process_gone(pid));

    let fixed = client
        .fix(Fix::ShutdownUnknownHolder {
            session: unrelated.clone(),
        })
        .await;
    assert_eq!(fixed, Ok(Reply::Done));
    wait_until("the unknown Holder to exit", || process_gone(pid)).await;
    let report = client.reconcile().await;
    assert!(unknown_holder(&report.unknown_holders, &unrelated).is_none());
}

async fn reboot(daemon: &mut Daemon, holder: i32) {
    daemon.kill();
    kill(holder, "-KILL");
    wait_until("the Holder to die", || process_gone(holder)).await;
}

fn findings_of(report: &ReconcileReport, repo: &std::path::Path) -> Vec<Finding> {
    report
        .repo(repo)
        .map(|repo| repo.findings.clone())
        .unwrap_or_default()
}

#[tokio::test]
async fn a_missing_worktree_is_flagged_and_recreating_it_resumes_the_agent() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = idle_session(&env, &mut client, "Fragile").await;
    let view = client.sessions[&id].clone();
    commit(&view.worktree, "work.rs", "work\n");
    reboot(&mut daemon, client.holder_pid(&id).unwrap()).await;
    std::fs::remove_dir_all(&view.worktree).unwrap();

    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let flagged = client
        .until(&id, "worktree missing", |view| view.flags.worktree_missing)
        .await;
    assert_eq!(flagged.phase, PhaseView::Suspended);
    let resumed = client
        .request(Request::Resume {
            session: id.clone(),
        })
        .await;
    assert!(
        matches!(resumed, Err(RequestError::Refused { .. })),
        "{resumed:?}"
    );
    let report = client.reconcile().await;
    assert_eq!(
        findings_of(&report, &view.repo),
        vec![Finding {
            problem: Problem::WorktreeMissing {
                session: id.clone()
            },
            fixes: vec![Fix::RecreateWorktree {
                session: id.clone()
            }],
        }]
    );

    let fixed = client
        .fix(Fix::RecreateWorktree {
            session: id.clone(),
        })
        .await;
    assert_eq!(fixed, Ok(Reply::Done));
    let active = client
        .until(&id, "resumed in the recreated Worktree", |view| {
            !view.flags.worktree_missing
                && view.phase == PhaseView::Active
                && view.holder_pid.is_some()
        })
        .await;
    assert_eq!(active.port_base, view.port_base);
    assert_eq!(
        std::fs::read_to_string(view.worktree.join("work.rs")).unwrap(),
        "work\n"
    );
    let mut pane = env.pane(&id, PANE).await;
    pane.wait_for_text("resume> conv-1").await;
    pane.type_line("env ORCH_PORT_BASE").await;
    pane.wait_for_text(&format!("ORCH_PORT_BASE={}", view.port_base.unwrap()))
        .await;
}

#[tokio::test]
async fn discarding_a_session_without_worktree_ends_only_the_record_and_keeps_its_branch() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let kept = running_session(&env, &mut client, "Keep branch").await;
    let gone = running_session(&env, &mut client, "Lose branch").await;
    let kept_view = client.sessions[&kept].clone();
    let gone_view = client.sessions[&gone].clone();
    commit(&kept_view.worktree, "work.rs", "work\n");
    let holders = [
        client.holder_pid(&kept).unwrap(),
        client.holder_pid(&gone).unwrap(),
    ];
    daemon.kill();
    for holder in holders {
        kill(holder, "-KILL");
        wait_until("the Holder to die", || process_gone(holder)).await;
    }
    std::fs::remove_dir_all(&kept_view.worktree).unwrap();
    std::fs::remove_dir_all(&gone_view.worktree).unwrap();
    git(&kept_view.repo, &["worktree", "prune"]);
    git(&kept_view.repo, &["branch", "-D", &gone_view.branch]);

    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let report = client.reconcile().await;
    let findings = findings_of(&report, &kept_view.repo);
    let fixes_for = |session: &SessionId| {
        findings
            .iter()
            .find(|finding| {
                finding.problem
                    == Problem::WorktreeMissing {
                        session: session.clone(),
                    }
            })
            .map(|finding| finding.fixes.clone())
    };
    assert_eq!(
        fixes_for(&kept),
        Some(vec![Fix::RecreateWorktree {
            session: kept.clone()
        }])
    );
    assert_eq!(
        fixes_for(&gone),
        Some(vec![Fix::DiscardRecord {
            session: gone.clone()
        }])
    );

    for session in [&kept, &gone] {
        let discarded = client
            .fix(Fix::DiscardRecord {
                session: session.clone(),
            })
            .await;
        assert_eq!(discarded, Ok(Reply::Done));
        client.until_removed(session).await;
    }
    assert_eq!(
        git(
            &kept_view.repo,
            &["log", "-1", "--format=%s", &kept_view.branch]
        ),
        "add work.rs"
    );
    let report = client.reconcile().await;
    let leftovers: Vec<LeftoverView> = report
        .repo(&kept_view.repo)
        .unwrap()
        .leftovers()
        .cloned()
        .collect();
    assert_eq!(
        leftovers,
        vec![LeftoverView::Branch {
            branch: kept_view.branch.clone()
        }]
    );
}

#[tokio::test]
async fn a_repo_that_moved_away_is_flagged_missing_and_can_be_forgotten() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Moving").await;
    let view = client.sessions[&id].clone();
    reboot(&mut daemon, client.holder_pid(&id).unwrap()).await;
    std::fs::rename(&view.repo, env.path("repos/elsewhere")).unwrap();

    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    client
        .until(&id, "Repo missing", |view| view.flags.repo_missing)
        .await;
    let report = client.reconcile().await;
    let repo = report.repo(&view.repo).expect("the Repo is reported");
    assert!(repo.missing);
    assert_eq!(
        repo.findings,
        vec![Finding {
            problem: Problem::RepoMissing,
            fixes: vec![Fix::ForgetRepo {
                repo: view.repo.clone()
            }],
        }]
    );

    let forgotten = client
        .fix(Fix::ForgetRepo {
            repo: view.repo.clone(),
        })
        .await;
    assert_eq!(forgotten, Ok(Reply::Done));
    client.until_removed(&id).await;
    assert!(client.reconcile().await.repo(&view.repo).is_none());
}

#[tokio::test]
async fn a_deleted_base_branch_is_flagged_blocks_landing_and_can_be_retargeted() {
    let env = Env::new();
    let repo = env.repo("app");
    git(&repo, &["branch", "develop"]);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let mut create = CreateSession::new(&repo, "Stacked work");
    create.base = Some("develop".into());
    let id = client.create(create).await;
    client
        .until(&id, "running", |view| view.holder_pid.is_some())
        .await;
    let mut pane = env.pane(&id, PANE).await;
    pane.hook(&hook("SessionStart", "")).await;
    let view = client
        .until(&id, "Idle", |view| view.agent == Some(AgentStateView::Idle))
        .await;
    commit(&view.worktree, "feature.rs", "feature\n");
    git(&repo, &["branch", "-D", "develop"]);

    let report = client.reconcile().await;
    assert_eq!(
        findings_of(&report, &repo),
        vec![Finding {
            problem: Problem::BaseMissing {
                session: id.clone(),
                base: "develop".into()
            },
            fixes: vec![Fix::Retarget {
                session: id.clone(),
                base: "main".into()
            }],
        }]
    );
    client
        .until(&id, "Base missing", |view| view.flags.base_missing)
        .await;
    let landed = client
        .request(Request::Land {
            session: id.clone(),
            landing: Landing::Squash {
                message: "Feature".into(),
            },
        })
        .await;
    assert!(
        matches!(&landed, Err(RequestError::Refused { message }) if message.contains("retarget")),
        "{landed:?}"
    );

    let retargeted = client
        .fix(Fix::Retarget {
            session: id.clone(),
            base: "main".into(),
        })
        .await;
    assert_eq!(retargeted, Ok(Reply::Done));
    let view = client
        .until(&id, "retargeted", |view| {
            !view.flags.base_missing && view.base == "main"
        })
        .await;
    assert!(view.flags.needs_rebase);
}

#[tokio::test]
async fn leftovers_are_listed_kept_across_reconciles_previewed_and_adopted_or_removed() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Owned").await;
    let repo = client.sessions[&id].repo.clone();
    let worktrees = repo.join(".orchestrator/worktrees");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "orch/abandoned",
            ".orchestrator/worktrees/abandoned",
        ],
    );
    std::fs::create_dir_all(worktrees.join("junk/deep")).unwrap();
    std::fs::write(worktrees.join("junk/deep/notes.txt"), "mine\n").unwrap();
    git(&repo, &["checkout", "-q", "-b", "orch/gone"]);
    commit(&repo, "gone.rs", "gone\n");
    git(&repo, &["checkout", "-q", "main"]);
    git(&repo, &["branch", "orch/lonely"]);
    let expected = vec![
        LeftoverView::Worktree {
            path: worktrees.join("abandoned"),
        },
        LeftoverView::Worktree {
            path: worktrees.join("junk"),
        },
        LeftoverView::Branch {
            branch: "orch/abandoned".into(),
        },
        LeftoverView::Branch {
            branch: "orch/gone".into(),
        },
        LeftoverView::Branch {
            branch: "orch/lonely".into(),
        },
    ];

    for _ in 0..2 {
        let report = client.reconcile().await;
        let listed: Vec<LeftoverView> = report.repo(&repo).unwrap().leftovers().cloned().collect();
        assert_eq!(listed, expected);
        let finding = &report.repo(&repo).unwrap().findings[0];
        assert_eq!(
            finding.fixes,
            vec![
                Fix::AdoptLeftover {
                    repo: repo.clone(),
                    leftover: expected[0].clone()
                },
                Fix::RemoveLeftover {
                    repo: repo.clone(),
                    leftover: expected[0].clone()
                },
            ]
        );
    }
    assert!(worktrees.join("junk/deep/notes.txt").exists());
    assert!(worktrees.join("abandoned").is_dir());

    let preview = |leftover: &LeftoverView| Request::LeftoverPreview {
        repo: repo.clone(),
        leftover: leftover.clone(),
    };
    assert_eq!(
        client.request(preview(&expected[1])).await,
        Ok(Reply::DiscardPreview {
            uncommitted: vec!["deep/notes.txt".into()],
            unlanded: Vec::new(),
        })
    );
    let Ok(Reply::DiscardPreview {
        uncommitted,
        unlanded,
    }) = client.request(preview(&expected[3])).await
    else {
        panic!("no preview of the gone Branch");
    };
    assert!(uncommitted.is_empty());
    assert_eq!(
        unlanded
            .iter()
            .map(|commit| commit.subject.as_str())
            .collect::<Vec<_>>(),
        vec!["add gone.rs"]
    );

    let Ok(Reply::Created { session: abandoned }) = client
        .fix(Fix::AdoptLeftover {
            repo: repo.clone(),
            leftover: expected[0].clone(),
        })
        .await
    else {
        panic!("adopting the abandoned Worktree failed");
    };
    let adopted = client
        .until(&abandoned, "adopted", |view| {
            view.phase == PhaseView::Suspended
        })
        .await;
    assert_eq!(adopted.branch, "orch/abandoned");
    assert_eq!(adopted.worktree, worktrees.join("abandoned"));
    assert!(adopted.port_base.is_some());

    let Ok(Reply::Created { session: lonely }) = client
        .fix(Fix::AdoptLeftover {
            repo: repo.clone(),
            leftover: expected[4].clone(),
        })
        .await
    else {
        panic!("adopting the lonely Branch failed");
    };
    let adopted = client
        .until(&lonely, "adopted", |view| {
            view.phase == PhaseView::Suspended
        })
        .await;
    assert_eq!(adopted.worktree, worktrees.join("lonely"));
    assert!(worktrees.join("lonely/README.md").exists());

    for leftover in [&expected[1], &expected[3]] {
        let removed = client
            .fix(Fix::RemoveLeftover {
                repo: repo.clone(),
                leftover: leftover.clone(),
            })
            .await;
        assert_eq!(removed, Ok(Reply::Done));
    }
    assert!(!worktrees.join("junk").exists());
    assert!(!git(&repo, &["branch", "--list", "orch/gone"]).contains("gone"));
    let report = client.reconcile().await;
    assert_eq!(report.repo(&repo).unwrap().leftovers().count(), 0);

    let resumed = client
        .request(Request::Resume {
            session: lonely.clone(),
        })
        .await;
    assert_eq!(resumed, Ok(Reply::Done));
    client
        .until(&lonely, "running", |view| {
            view.phase == PhaseView::Active && view.holder_pid.is_some()
        })
        .await;
}

#[tokio::test]
async fn a_pr_merged_while_the_daemon_was_down_is_cleaned_up_on_start() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = open_pr(&env, &mut client).await;
    let view = client.sessions[&id].clone();
    reboot(&mut daemon, client.holder_pid(&id).unwrap()).await;
    env.gh_reports(r#"{"state":"MERGED"}"#);

    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    if client.session_list().await.iter().any(|view| view.id == id) {
        client.until_removed(&id).await;
    }
    assert!(!view.worktree.exists());
    assert!(!git(&view.repo, &["branch", "--list", &view.branch]).contains(&view.branch));
}

#[tokio::test]
async fn a_landing_whose_cleanup_failed_is_finished_by_reconciliation() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, view) = land_with_stuck_cleanup(&env, &mut client).await;
    assert!(view.worktree.exists());

    let report = client.reconcile().await;
    assert!(
        report.repaired.contains(&Repair::FinishedCleanup {
            session: id.clone()
        }),
        "{report:?}"
    );
    assert!(!view.worktree.exists());
    assert!(!git(&view.repo, &["branch", "--list", &view.branch]).contains(&view.branch));
    let report = client.reconcile().await;
    assert!(findings_of(&report, &view.repo).is_empty(), "{report:?}");
}

#[tokio::test]
async fn ended_sessions_leave_the_session_list() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = idle_session(&env, &mut client, "Short lived").await;
    let discarded = client
        .request(Request::Discard {
            session: id.clone(),
        })
        .await;
    assert_eq!(discarded, Ok(Reply::Done));
    client.until_removed(&id).await;
    let mut other = env.client().await;
    assert!(other.session_list().await.is_empty());
}

#[tokio::test]
async fn reconciliation_repeats_periodically_without_a_request() {
    let env = Env::new();
    let _daemon = env
        .start_daemon_args(
            std::time::Duration::from_secs(600),
            &["--reconcile-ms", "200"],
        )
        .await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Watched").await;
    let view = client.sessions[&id].clone();
    std::fs::remove_dir_all(&view.worktree).unwrap();

    client
        .until(&id, "worktree missing", |view| view.flags.worktree_missing)
        .await;
    let session = id.clone();
    wait_for_report(&mut client, move |report| {
        report.findings().any(|finding| {
            finding.problem
                == Problem::WorktreeMissing {
                    session: session.clone(),
                }
        })
    })
    .await;
}

async fn wait_for_report(client: &mut TestClient, wanted: impl Fn(&ReconcileReport) -> bool) {
    let deadline = std::time::Instant::now() + WAIT;
    while !client.reports.iter().any(&wanted) {
        assert!(
            std::time::Instant::now() < deadline,
            "no such report broadcast"
        );
        let _ = tokio::time::timeout(std::time::Duration::from_millis(100), client.next()).await;
    }
}

#[tokio::test]
async fn a_holder_reports_its_sessions_worktree_base_and_port_block() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Introduce").await;
    let view = client.sessions[&id].clone();

    let mut holder = orch_holder::HolderClient::connect(&env.holder_socket(&id))
        .await
        .unwrap();
    let hello = holder.attach().await.unwrap();

    assert_eq!(hello.cwd.as_deref(), Some(view.worktree.as_path()));
    assert_eq!(hello.base.as_deref(), Some("main"));
    assert_eq!(
        hello.port_block,
        Some(orch_config::PortBlock {
            base: view.port_base.unwrap(),
            size: view.port_size.unwrap(),
        })
    );
}

async fn open_pr(env: &Env, client: &mut TestClient) -> SessionId {
    let repo = env.repo("app");
    env.remote(&repo);
    env.fake_gh();
    let (id, _pane) = idle_session(env, client, "Add caching").await;
    let view = client.sessions[&id].clone();
    commit(&view.worktree, "cache.rs", "cache\n");
    let opened = client
        .request(Request::Land {
            session: id.clone(),
            landing: Landing::Pr {
                title: "Add caching".into(),
                body: String::new(),
            },
        })
        .await;
    assert!(matches!(opened, Ok(Reply::PrOpened { .. })), "{opened:?}");
    id
}

#[tokio::test]
async fn an_adopted_holder_whose_branch_has_an_open_pr_is_pr_open_again() {
    let env = Env::new();
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = open_pr(&env, &mut client).await;
    let view = client
        .until(&id, "PR open", |view| view.phase == PhaseView::PrOpen)
        .await;

    daemon.kill();
    env.lose_state_db();
    env.gh_lists(r#"[{"number":42}]"#);
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let adopted = client
        .until(&id, "adopted with its PR", |view| {
            view.flags.recovered && view.phase == PhaseView::PrOpen
        })
        .await;
    assert_eq!(adopted.flags.pr_number, Some(42));
    assert!(
        env.gh_calls()
            .contains(&format!("pr\nlist\n--head\n{}\n", view.branch)),
        "{}",
        env.gh_calls()
    );
}

#[tokio::test]
async fn an_adopted_holder_keeps_its_port_block_and_a_clash_is_reported_with_fixes() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let first = running_session(&env, &mut client, "First").await;
    let first_view = client.sessions[&first].clone();
    let taken = first_view.port_base.unwrap();
    let repo = first_view.repo.clone();
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "orch/by-hand",
            ".orchestrator/worktrees/by-hand",
        ],
    );
    let by_hand = SessionId::parse("by-hand-holder").unwrap();
    env.hold_with(
        by_hand.as_str(),
        &repo.join(".orchestrator/worktrees/by-hand"),
        &["--port-base", &taken.to_string(), "--port-size", "10"],
    );

    let report = client.reconcile().await;
    let adopted = client
        .until(&by_hand, "adopted", |view| view.holder_pid.is_some())
        .await;
    assert_eq!(adopted.port_base, Some(taken));
    let clash = Finding {
        problem: Problem::PortBlockClash {
            session: by_hand.clone(),
            other: first.clone(),
        },
        fixes: vec![
            Fix::ReassignPortBlock {
                session: by_hand.clone(),
            },
            Fix::ReassignPortBlock {
                session: first.clone(),
            },
        ],
    };
    assert!(findings_of(&report, &repo).contains(&clash), "{report:#?}");

    let third = running_session(&env, &mut client, "Third").await;
    let third_base = client.sessions[&third].port_base.unwrap();
    assert!(third_base >= taken + 10, "{third_base} overlaps {taken}");

    let reassigned = client
        .fix(Fix::ReassignPortBlock {
            session: by_hand.clone(),
        })
        .await;
    assert_eq!(reassigned, Ok(Reply::Done));
    let moved = client
        .until(&by_hand, "running on a new block", |view| {
            view.port_base.is_some_and(|base| base != taken)
                && view
                    .holder_pid
                    .is_some_and(|pid| Some(pid) != adopted.holder_pid)
        })
        .await;
    let moved_base = moved.port_base.unwrap();
    assert_ne!(moved_base, third_base);
    let mut pane = env.pane(&by_hand, PANE).await;
    pane.type_line("env ORCH_PORT_BASE").await;
    pane.wait_for_text(&format!("ORCH_PORT_BASE={moved_base}"))
        .await;
    let report = client.reconcile().await;
    assert!(!findings_of(&report, &repo).contains(&clash));
}

async fn land_with_stuck_cleanup(
    env: &Env,
    client: &mut TestClient,
) -> (SessionId, orch_protocol::SessionView) {
    use std::os::unix::fs::PermissionsExt;
    let (id, _pane) = idle_session(env, client, "Stubborn").await;
    let view = client.sessions[&id].clone();
    let locked = view.worktree.join("locked");
    std::fs::create_dir_all(&locked).unwrap();
    std::fs::write(locked.join("file.txt"), "stuck\n").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    let landed = client
        .request(Request::Land {
            session: id.clone(),
            landing: Landing::Squash {
                message: "Stubborn".into(),
            },
        })
        .await;
    assert!(
        matches!(&landed, Ok(Reply::Landed { warning: Some(warning), .. }) if warning.contains("removing the Worktree")),
        "{landed:?}"
    );
    client.until_removed(&id).await;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    (id, view)
}

#[tokio::test]
async fn an_unfinished_cleanup_never_touches_a_worktree_changed_since_the_session_ended() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, view) = land_with_stuck_cleanup(&env, &mut client).await;
    std::fs::write(view.worktree.join("afterwards.txt"), "keep me\n").unwrap();

    let report = client.reconcile().await;
    assert!(
        !report.repaired.contains(&Repair::FinishedCleanup {
            session: id.clone()
        }),
        "{report:?}"
    );
    assert_eq!(
        std::fs::read_to_string(view.worktree.join("afterwards.txt")).unwrap(),
        "keep me\n"
    );
    let leftovers: Vec<LeftoverView> = report
        .repo(&view.repo)
        .unwrap()
        .leftovers()
        .cloned()
        .collect();
    assert!(
        leftovers.contains(&LeftoverView::Worktree {
            path: view.worktree.clone()
        }),
        "{leftovers:?}"
    );
}

#[tokio::test]
async fn an_unfinished_cleanup_never_touches_a_worktree_made_later_at_the_same_slug() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, view) = land_with_stuck_cleanup(&env, &mut client).await;
    std::fs::remove_dir_all(&view.worktree).unwrap();
    git(&view.repo, &["worktree", "prune"]);
    git(
        &view.repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "by-hand",
            view.worktree.to_str().unwrap(),
        ],
    );
    std::fs::write(view.worktree.join("mine.txt"), "mine\n").unwrap();

    let report = client.reconcile().await;
    assert_eq!(
        std::fs::read_to_string(view.worktree.join("mine.txt")).unwrap(),
        "mine\n"
    );
    assert_eq!(
        git(&view.worktree, &["branch", "--show-current"]),
        "by-hand"
    );
    assert!(
        report
            .repo(&view.repo)
            .unwrap()
            .leftovers()
            .any(|leftover| *leftover
                == LeftoverView::Worktree {
                    path: view.worktree.clone()
                }),
        "{report:?}"
    );
    let _ = id;
}

#[tokio::test]
async fn an_interrupted_cleanup_runs_the_teardown_it_missed() {
    let env = Env::new();
    let repo = env.repo("app");
    let gate = env.path("gate");
    let log = env.path("teardown.log");
    env.write_config(&format!(
        "[repos.{repo:?}]\nteardown = \"[ -f {} ] || exec sleep 600; echo ran >> {}\"\n",
        gate.display(),
        log.display()
    ));
    let mut daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (id, _pane) = idle_session(&env, &mut client, "Interrupted").await;
    let view = client.sessions[&id].clone();
    std::fs::write(view.worktree.join("feature.rs"), "feature\n").unwrap();
    let mut lander = env.client().await;
    let session = id.clone();
    let landing = tokio::spawn(async move {
        let _ = lander
            .client
            .request(Request::Land {
                session,
                landing: Landing::Squash {
                    message: "Interrupted".into(),
                },
            })
            .await;
    });
    wait_until("the Teardown script to start", || {
        env.processes().iter().any(|pid| {
            std::fs::read(format!("/proc/{pid}/cmdline"))
                .is_ok_and(|cmdline| cmdline.starts_with(b"sleep\x00600"))
        })
    })
    .await;
    daemon.kill();
    landing.abort();
    std::fs::write(&gate, "").unwrap();

    let _daemon = env.start_daemon().await;
    wait_until("the cleanup to finish", || {
        !view.worktree.exists()
            && !git(&repo, &["branch", "--list", &view.branch]).contains(&view.branch)
    })
    .await;
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "ran\n");
}
