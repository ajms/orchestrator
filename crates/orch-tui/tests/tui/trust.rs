use orch_protocol::{Reply, Request, RequestError};

use crate::common::*;

fn untrusted() -> Result<Reply, RequestError> {
    Err(RequestError::Untrusted {
        repo: "/home/me/webshop".into(),
        hash: "abc123".into(),
        items: vec!["Teardown script: make clean".into()],
    })
}

fn discard_answered_untrusted() -> Harness {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "doomed")]);
    tui.daemon().script_reply(Ok(Reply::DiscardPreview {
        uncommitted: Vec::new(),
        unlanded: Vec::new(),
    }));
    tui.daemon().script_reply(untrusted());
    tui.command("discard");
    tui.keys("y");
    tui
}

fn tail(tui: &mut Harness, count: usize) -> Vec<Request> {
    let requests = tui.daemon().requests();
    requests[requests.len() - count..].to_vec()
}

#[test]
fn an_untrusted_discard_asks_for_trust_and_retries_after_approval() {
    let mut tui = discard_answered_untrusted();
    let screen = tui.screen();
    assert!(screen.contains("Trust"), "{screen}");
    assert!(screen.contains("/home/me/webshop"), "{screen}");
    assert!(screen.contains("Teardown script: make clean"), "{screen}");

    tui.keys("y");

    assert_eq!(
        tail(&mut tui, 2),
        [
            Request::ApproveTrust {
                repo: "/home/me/webshop".into(),
                hash: "abc123".into(),
            },
            Request::Discard {
                session: id("doomed"),
                skip_teardown: false,
            },
        ]
    );
}

#[test]
fn an_untrusted_teardown_can_be_skipped_from_the_trust_prompt() {
    let mut tui = discard_answered_untrusted();
    assert!(tui.screen().contains("skip the Teardown"));

    tui.keys("s");

    assert_eq!(
        tail(&mut tui, 1),
        [Request::Discard {
            session: id("doomed"),
            skip_teardown: true,
        }]
    );
    assert!(!tui.screen().contains("make clean"));
}

#[test]
fn an_untrusted_resume_asks_for_trust_and_resumes_after_approval() {
    let mut tui = Harness::new();
    let mut suspended = session("webshop", "paused");
    suspended.phase = orch_protocol::PhaseView::Suspended;
    suspended.agent = None;
    tui.sessions(vec![suspended]);
    tui.daemon().script_reply(untrusted());
    tui.command("resume");
    assert!(tui.screen().contains("Teardown script: make clean"));
    assert!(!tui.screen().contains("skip the Teardown"));

    tui.keys("y");

    assert_eq!(
        tail(&mut tui, 2),
        [
            Request::ApproveTrust {
                repo: "/home/me/webshop".into(),
                hash: "abc123".into(),
            },
            Request::Resume {
                session: id("paused"),
            },
        ]
    );
}

#[test]
fn declining_trust_retries_nothing() {
    let mut tui = discard_answered_untrusted();
    let before = tui.daemon().requests().len();
    tui.keys("n");
    assert_eq!(tui.daemon().requests().len(), before);
    assert!(!tui.screen().contains("make clean"));
}
