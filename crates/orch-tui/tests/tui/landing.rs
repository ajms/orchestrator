use crossterm::event::KeyCode;
use orch_protocol::{
    AgentStateView, CommitView, Landing, LandingMode, Reply, Request, RequestError,
};
use orch_tui::{Effect, Event, FileDiff, ReviewData, ReviewPurpose};
use ratatui::style::Color;

use crate::common::*;

fn statusline(tui: &mut Harness) -> String {
    tui.lines().pop().unwrap()
}

fn last_request(tui: &mut Harness) -> Request {
    tui.daemon().requests().pop().unwrap()
}

fn drafted(title: &str, body: &str) -> Result<Reply, RequestError> {
    Ok(Reply::Drafted {
        title: title.into(),
        body: body.into(),
    })
}

fn landing(tui: &mut Harness) -> Vec<Landing> {
    tui.daemon()
        .requests()
        .into_iter()
        .filter_map(|request| match request {
            Request::Land { landing, .. } => Some(landing),
            _ => None,
        })
        .collect()
}

#[test]
fn land_is_blocked_while_the_agent_is_working_or_needs_input() {
    for state in [AgentStateView::Working, AgentStateView::NeedsInput] {
        let mut tui = Harness::new();
        tui.sessions(vec![with_agent(session("webshop", "busy"), state)]);
        tui.command("land");
        let status = statusline(&mut tui);
        assert!(status.contains("Landing is blocked"), "{status}");
        assert!(!tui.screen().contains("squash"));
    }
}

#[test]
fn the_land_popup_shows_the_agents_draft_and_lands_a_squash() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "done")]);
    tui.daemon().script_reply(drafted(
        "Read the timeout from config",
        "No more magic numbers.",
    ));
    tui.command("land");

    assert_eq!(
        last_request(&mut tui),
        Request::Draft {
            session: id("done"),
            mode: LandingMode::Squash
        }
    );
    let screen = tui.screen();
    assert!(screen.contains("into main"), "{screen}");
    assert!(screen.contains("Read the timeout from config"), "{screen}");
    assert!(screen.contains("No more magic numbers."), "{screen}");
    assert_eq!(tui.background_of("squash onto main"), Color::Green);

    tui.keys(" (#12)");
    tui.press(KeyCode::Enter);
    assert_eq!(
        landing(&mut tui),
        vec![Landing::Squash {
            message: "Read the timeout from config\n\nNo more magic numbers. (#12)".into()
        }]
    );
    assert!(!tui.screen().contains("squash onto main"));
}

#[test]
fn tab_switches_to_a_pr_with_its_own_draft() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "done")]);
    tui.daemon().script_reply(drafted("squash title", ""));
    tui.daemon()
        .script_reply(drafted("PR title", "PR body\nmore"));
    tui.command("land");
    tui.press(KeyCode::Tab);

    assert_eq!(
        last_request(&mut tui),
        Request::Draft {
            session: id("done"),
            mode: LandingMode::Pr
        }
    );
    assert_eq!(tui.background_of("push + PR"), Color::Green);
    tui.press(KeyCode::Enter);
    assert_eq!(
        landing(&mut tui),
        vec![Landing::Pr {
            title: "PR title".into(),
            body: "PR body\nmore".into()
        }]
    );
}

#[test]
fn an_edited_message_is_kept_when_switching_targets() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "done")]);
    tui.daemon().script_reply(drafted("draft", ""));
    tui.command("land");
    tui.keys("!");
    let drafts = tui.daemon().requests().len();
    tui.press(KeyCode::Tab);
    assert_eq!(tui.daemon().requests().len(), drafts);
    assert!(tui.screen().contains("draft!"));
}

#[test]
fn ctrl_g_edits_the_landing_message_in_the_editor() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "done")]);
    tui.daemon().script_reply(drafted("draft", ""));
    tui.command("land");
    tui.ctrl('g');
    assert!(tui.take_effects().contains(&Effect::EditText {
        text: "draft".into()
    }));
    tui.send(Event::EditorClosed(Ok(
        "Better subject\n\nBetter body\n".into()
    )));
    tui.press(KeyCode::Enter);
    assert_eq!(
        landing(&mut tui),
        vec![Landing::Squash {
            message: "Better subject\n\nBetter body".into()
        }]
    );
}

#[test]
fn the_landing_outcome_is_reported() {
    let outcomes = [
        (
            Ok(Reply::Landed {
                commit: "abc1234".into(),
                warning: None,
            }),
            "Landed as abc1234",
        ),
        (
            Ok(Reply::Landed {
                commit: "abc1234".into(),
                warning: Some("the Teardown script failed".into()),
            }),
            "the Teardown script failed",
        ),
        (
            Ok(Reply::PrOpened {
                number: 61,
                committed_changes: false,
            }),
            "opened PR #61",
        ),
        (
            Ok(Reply::PrOpened {
                number: 62,
                committed_changes: true,
            }),
            "uncommitted changes were committed with the PR title",
        ),
        (
            Err(RequestError::Conflict {
                paths: vec!["src/a.rs".into()],
            }),
            "conflict in: src/a.rs; the rebase was handed to the Agent",
        ),
    ];
    for (reply, expected) in outcomes {
        let mut tui = Harness::new();
        tui.sessions(vec![session("webshop", "done")]);
        tui.daemon().script_reply(drafted("draft", ""));
        tui.daemon().script_reply(reply);
        tui.command("land");
        tui.press(KeyCode::Enter);
        let status = statusline(&mut tui);
        assert!(status.contains(expected), "{status}");
    }
}

#[test]
fn land_works_from_the_review() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "done")]);
    tui.keys("d");
    tui.send(Event::Review {
        session: id("done"),
        purpose: ReviewPurpose::BuiltIn,
        result: Ok(ReviewData {
            merge_base: "b".into(),
            tree: "t".into(),
            files: vec![FileDiff {
                path: "a.rs".into(),
                lines: vec!["@@ -0,0 +1 @@".into(), "+a".into()],
            }],
        }),
    });
    tui.command("land");
    assert!(tui.screen().contains("push + PR"));
}

#[test]
fn discard_previews_what_would_be_lost_before_a_one_key_confirmation() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "doomed")]);
    tui.daemon().script_reply(Ok(Reply::DiscardPreview {
        uncommitted: vec!["src/login.rs".into(), "src/new.rs".into()],
        unlanded: vec![CommitView {
            id: "1a2b3c4d5e6f".into(),
            subject: "Read timeout from config".into(),
        }],
    }));
    tui.command("discard");
    assert_eq!(
        last_request(&mut tui),
        Request::DiscardPreview {
            session: id("doomed")
        }
    );

    let screen = tui.screen();
    assert!(screen.contains("Discard webshop / doomed?"), "{screen}");
    assert!(screen.contains("src/login.rs"), "{screen}");
    assert!(screen.contains("src/new.rs"), "{screen}");
    assert!(
        screen.contains("1a2b3c4 Read timeout from config"),
        "{screen}"
    );

    tui.keys("y");
    assert_eq!(
        last_request(&mut tui),
        Request::Discard {
            session: id("doomed")
        }
    );
    assert!(!tui.screen().contains("src/new.rs"));
}

#[test]
fn any_other_key_cancels_the_discard() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "doomed")]);
    tui.daemon().script_reply(Ok(Reply::DiscardPreview {
        uncommitted: vec![],
        unlanded: vec![],
    }));
    tui.command("discard");
    assert!(tui.screen().contains("nothing uncommitted"));
    tui.keys("x");
    assert!(!tui.screen().contains("Discard webshop"));
    assert!(
        !tui.daemon()
            .requests()
            .iter()
            .any(|request| matches!(request, Request::Discard { .. }))
    );
}

#[test]
fn a_closed_pr_can_be_abandoned_back_to_active() {
    let mut view = in_phase(
        session("webshop", "rejected"),
        orch_protocol::PhaseView::PrOpen,
    );
    view.agent = Some(AgentStateView::Idle);
    view.flags.pr_number = Some(9);
    view.flags.pr = Some(orch_protocol::PrView {
        checks: orch_protocol::PrChecksView::None,
        review: orch_protocol::PrReviewView::None,
        new_comments: 0,
        closed: true,
    });
    let mut tui = Harness::new();
    tui.sessions(vec![view]);
    assert!(tui.sidebar_lines().join("\n").contains(":abandon"));

    tui.command("abandon");
    assert_eq!(
        last_request(&mut tui),
        Request::AbandonPr {
            session: id("rejected")
        }
    );
}

#[test]
fn input_dropped_during_a_landing_is_explained() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "done")]);
    tui.pane(
        "done",
        orch_protocol::FromDaemon::InputDropped {
            reason: "a Landing is in progress".into(),
        },
    );
    assert!(statusline(&mut tui).contains("a Landing is in progress"));
}
