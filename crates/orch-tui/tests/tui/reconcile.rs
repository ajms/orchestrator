use crossterm::event::KeyCode;
use orch_protocol::{
    CommitView, Finding, Fix, FromDaemon, LeftoverView, Problem, ReconcileReport, Reply,
    RepoReport, Request,
};
use orch_tui::Event;

use crate::common::*;

fn report() -> ReconcileReport {
    ReconcileReport {
        repos: vec![
            RepoReport {
                repo: "/home/me/webshop".into(),
                missing: false,
                findings: vec![
                    Finding {
                        problem: Problem::WorktreeMissing {
                            session: id("first"),
                        },
                        fixes: vec![
                            Fix::RecreateWorktree {
                                session: id("first"),
                            },
                            Fix::DiscardRecord {
                                session: id("first"),
                            },
                        ],
                    },
                    Finding {
                        problem: Problem::Leftover {
                            leftover: LeftoverView::Branch {
                                branch: "orch/abandoned-idea".into(),
                            },
                        },
                        fixes: vec![
                            Fix::AdoptLeftover {
                                repo: "/home/me/webshop".into(),
                                leftover: LeftoverView::Branch {
                                    branch: "orch/abandoned-idea".into(),
                                },
                            },
                            Fix::RemoveLeftover {
                                repo: "/home/me/webshop".into(),
                                leftover: LeftoverView::Branch {
                                    branch: "orch/abandoned-idea".into(),
                                },
                            },
                        ],
                    },
                    Finding {
                        problem: Problem::BaseMissing {
                            session: id("first"),
                            base: "orch/landed".into(),
                        },
                        fixes: vec![Fix::Retarget {
                            session: id("first"),
                            base: "main".into(),
                        }],
                    },
                    Finding {
                        problem: Problem::PortBlockClash {
                            session: id("first"),
                            other: id("second"),
                        },
                        fixes: vec![Fix::ReassignPortBlock {
                            session: id("first"),
                        }],
                    },
                ],
            },
            RepoReport {
                repo: "/home/me/vanished".into(),
                missing: true,
                findings: vec![Finding {
                    problem: Problem::RepoMissing,
                    fixes: vec![Fix::ForgetRepo {
                        repo: "/home/me/vanished".into(),
                    }],
                }],
            },
        ],
        unknown_holders: vec![Finding {
            problem: Problem::UnknownHolder {
                session: id("ghost"),
                holder_pid: 4242,
                cwd: None,
            },
            fixes: vec![Fix::ShutdownUnknownHolder {
                session: id("ghost"),
            }],
        }],
        repaired: vec![],
    }
}

fn reconciling() -> Harness {
    let mut tui = Harness::new();
    let mut other = session("webshop", "second");
    other.branch = "orch/second".into();
    tui.sessions(vec![session("webshop", "first"), other]);
    tui.daemon().script_reply(Ok(Reply::Reconciled {
        report: Box::new(report()),
    }));
    tui.command("reconcile");
    tui
}

fn fixes(tui: &mut Harness) -> Vec<Fix> {
    tui.daemon()
        .requests()
        .into_iter()
        .filter_map(|request| match request {
            Request::Fix { fix } => Some(fix),
            _ => None,
        })
        .collect()
}

#[test]
fn reconcile_lists_findings_per_repo_with_their_fixes() {
    let mut tui = reconciling();
    assert_eq!(tui.daemon().requests().pop(), Some(Request::Reconcile));
    let screen = tui.screen();
    for expected in [
        "webshop",
        "Worktree missing: first",
        "Recreate the Worktree",
        "Discard the Session record",
        "Leftover Branch orch/abandoned-idea",
        "Base orch/landed missing: first",
        "Port block clash: first and second",
        "Reassign the Port block",
        "vanished",
        "Repo missing",
        "Forget the Repo",
        "Unknown Holder ghost (pid 4242)",
    ] {
        assert!(
            screen.contains(expected),
            "{expected:?} missing in\n{screen}"
        );
    }
}

#[test]
fn enter_applies_the_selected_fix() {
    let mut tui = reconciling();
    tui.press(KeyCode::Enter);
    tui.keys("j");
    tui.press(KeyCode::Enter);
    assert_eq!(
        fixes(&mut tui),
        vec![
            Fix::RecreateWorktree {
                session: id("first")
            },
            Fix::DiscardRecord {
                session: id("first")
            },
        ]
    );
}

#[test]
fn removing_a_leftover_previews_what_is_lost_and_needs_confirmation() {
    let mut tui = reconciling();
    tui.keys("jjj");
    tui.daemon().script_reply(Ok(Reply::DiscardPreview {
        uncommitted: vec![],
        unlanded: vec![CommitView {
            id: "abcdef123456".into(),
            subject: "Stray work".into(),
        }],
    }));
    tui.press(KeyCode::Enter);
    let leftover = LeftoverView::Branch {
        branch: "orch/abandoned-idea".into(),
    };
    assert_eq!(
        tui.daemon().requests().pop(),
        Some(Request::LeftoverPreview {
            repo: "/home/me/webshop".into(),
            leftover: leftover.clone(),
        })
    );
    assert!(tui.screen().contains("abcdef1 Stray work"));
    assert!(fixes(&mut tui).is_empty());

    tui.keys("y");
    assert_eq!(
        fixes(&mut tui),
        vec![Fix::RemoveLeftover {
            repo: "/home/me/webshop".into(),
            leftover
        }]
    );
}

#[test]
fn retarget_lets_the_user_pick_a_branch() {
    let mut tui = reconciling();
    tui.keys("jjjj");
    tui.press(KeyCode::Enter);
    assert!(tui.screen().contains("Retarget first onto"));
    tui.press(KeyCode::Right);
    tui.press(KeyCode::Enter);
    assert_eq!(
        fixes(&mut tui),
        vec![Fix::Retarget {
            session: id("first"),
            base: "orch/second".into()
        }]
    );
}

#[test]
fn a_broadcast_report_with_findings_is_flagged_in_the_statusline() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first")]);
    assert!(!statusline(&mut tui).contains("findings"));
    tui.send(Event::Daemon(FromDaemon::Reconciled {
        report: Box::new(report()),
    }));
    let status = statusline(&mut tui);
    assert!(status.contains("6 findings"), "{status}");
    tui.send(Event::Daemon(FromDaemon::Reconciled {
        report: Box::new(ReconcileReport::default()),
    }));
    assert!(!statusline(&mut tui).contains("findings"));
}

#[test]
fn q_closes_the_reconciliation_view() {
    let mut tui = reconciling();
    tui.keys("q");
    assert!(!tui.screen().contains("Recreate the Worktree"));
}

#[test]
fn a_successful_fix_refreshes_the_report() {
    let mut tui = reconciling();
    tui.press(KeyCode::Enter);
    assert_eq!(tui.daemon().requests().pop(), Some(Request::Reconcile));
}

#[test]
fn a_missing_repo_without_sessions_still_shows_in_the_sidebar() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first")]);
    tui.send(Event::Daemon(FromDaemon::Reconciled {
        report: Box::new(report()),
    }));
    assert!(
        tui.sidebar_lines()
            .join("\n")
            .contains("vanished (missing)")
    );
}

#[test]
fn the_retarget_picker_starts_on_the_suggested_base() {
    let mut tui = reconciling();
    tui.keys("jjjj");
    tui.press(KeyCode::Enter);
    assert!(tui.screen().contains("◂ main ▸"));
    tui.press(KeyCode::Enter);
    assert_eq!(
        fixes(&mut tui),
        vec![Fix::Retarget {
            session: id("first"),
            base: "main".into()
        }]
    );
}
