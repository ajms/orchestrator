use orch_core::{Retarget, SessionId, retarget};

#[test]
fn sessions_stacked_on_a_landed_branch_are_retargeted_to_its_base() {
    let stacked = SessionId("b".into());
    let unrelated = SessionId("c".into());
    let bases = [(&stacked, "orch/a"), (&unrelated, "main")];

    assert_eq!(
        retarget(bases, "orch/a", "main"),
        [Retarget {
            session: stacked.clone(),
            base: "main".into()
        }]
    );
}

#[test]
fn nothing_is_retargeted_when_no_session_is_stacked() {
    let session = SessionId("b".into());
    assert!(retarget([(&session, "main")], "orch/a", "main").is_empty());
}
