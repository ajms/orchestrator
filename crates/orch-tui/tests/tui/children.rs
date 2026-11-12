use orch_protocol::{AgentStateView, Reply, SessionView};

use crate::common::*;

fn child_of(parent: &str, slug: &str) -> SessionView {
    let mut view = session("webshop", slug);
    view.parent = Some(id(parent));
    view.base = format!("orch/{parent}");
    view
}

fn family() -> Harness {
    let mut tui = Harness::new();
    tui.sessions(vec![
        child_of("lead", "part-a"),
        session("webshop", "lead"),
        session("webshop", "other"),
        child_of("part-a", "part-a1"),
    ]);
    tui
}

fn row(tui: &mut Harness, needle: &str) -> usize {
    tui.sidebar_lines()
        .iter()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle} not in the sidebar"))
}

#[test]
fn child_sessions_nest_under_their_parent_in_the_sidebar() {
    let mut tui = family();
    let (lead, a, a1, other) = (
        row(&mut tui, "lead"),
        row(&mut tui, "part-a "),
        row(&mut tui, "part-a1"),
        row(&mut tui, "other"),
    );
    assert!(
        lead < a && a < a1 && a1 < other,
        "{:?}",
        tui.sidebar_lines()
    );
    assert!(tui.sidebar_line_with("part-a1").contains("└"));
    assert!(tui.sidebar_line_with("lead").contains("▾"));
}

#[test]
fn a_collapsed_parent_hides_its_children_and_shows_their_most_urgent_mark() {
    let mut tui = family();
    let mut needy = with_agent(child_of("part-a", "part-a1"), AgentStateView::NeedsInput);
    needy.flags.unseen = true;
    tui.changed(needy);
    let mut unseen = child_of("lead", "part-a");
    unseen.flags.unseen = true;
    tui.changed(unseen);

    tui.keys("za");
    assert!(!in_sidebar(&mut tui, "part-a"));
    let lead = tui.sidebar_line_with("lead");
    assert!(lead.contains("▸"), "{lead}");
    assert!(lead.contains("+2"), "{lead}");
    assert!(lead.contains("Needs input"), "{lead}");

    tui.keys("j");
    assert_eq!(shown(&mut tui).as_deref(), Some("other"));
    tui.keys("kza");
    assert!(in_sidebar(&mut tui, "part-a1"));
}

#[test]
fn done_and_land_failed_show_on_the_child_row() {
    let mut tui = family();
    let mut done = child_of("lead", "part-a");
    done.done_summary = Some("Built it".into());
    tui.changed(done);
    let mut failed = child_of("part-a", "part-a1");
    failed.land_failed = Some("conflict in a.rs".into());
    tui.changed(failed);
    assert!(tui.sidebar_line_with("part-a ").contains("Done"));
    assert!(tui.sidebar_line_with("part-a1").contains("Land failed"));
}

#[test]
fn discarding_a_parent_lists_the_children_discarded_with_it() {
    let mut tui = family();
    tui.daemon().script_reply(Ok(Reply::DiscardPreview {
        uncommitted: Vec::new(),
        unlanded: Vec::new(),
        branch_kept: false,
        children: vec!["part-a1".into(), "part-a".into()],
    }));
    tui.command("discard");
    let screen = tui.screen();
    assert!(
        screen.contains("Child Sessions are Discarded too"),
        "{screen}"
    );
    assert!(screen.contains("part-a1"), "{screen}");
}

#[test]
fn child_starts_the_new_session_form_on_the_selected_session() {
    let mut tui = family();
    tui.command("child");
    assert!(tui.screen().contains("child of lead"), "{}", tui.screen());
    tui.keys("Split the work");
    tui.ctrl('s');
    let created = create_requests(&mut tui);
    assert_eq!(created.len(), 1, "{:?}", tui.daemon().requests());
    assert_eq!(created[0].parent, Some(id("lead")));
    assert_eq!(created[0].branch, None);
}

#[test]
fn the_plan_shows_the_child_sessions_working_on_a_task() {
    use orch_core::{Attachment, PlanOp};
    let mut tui = family();
    let mut lead = session("webshop", "lead");
    lead.plan
        .apply(PlanOp::Add {
            title: "Split".into(),
            after: None,
            blocked_by: Vec::new(),
            link: None,
        })
        .unwrap();
    lead.plan
        .apply(PlanOp::Attach {
            task: 1,
            attachment: Attachment::ChildSession("part-a".into()),
        })
        .unwrap();
    tui.changed(lead);
    let mut done = child_of("lead", "part-a");
    done.done_summary = Some("Built it".into());
    tui.changed(done);

    tui.keys("p");
    let line = tui.line_with("⇲ part-a");
    assert!(line.contains("Done"), "{line}");
}
