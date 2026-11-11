use crossterm::event::KeyCode;
use orch_core::{Attachment, Plan, PlanOp, TaskStatus};
use orch_protocol::{SubagentStateView, SubagentView};

use crate::common::*;

fn planned() -> Plan {
    let mut plan = Plan::default();
    for (title, blocked_by) in [
        ("Design the schema", vec![]),
        ("Write the migration", vec![1]),
        ("Old idea", vec![]),
        ("Wire the CLI", vec![2]),
    ] {
        plan.apply(PlanOp::Add {
            title: title.into(),
            after: None,
            blocked_by,
            link: None,
        })
        .unwrap();
    }
    let ops = [
        PlanOp::Status {
            task: 1,
            status: TaskStatus::Done,
        },
        PlanOp::Status {
            task: 2,
            status: TaskStatus::InProgress,
        },
        PlanOp::Status {
            task: 3,
            status: TaskStatus::Dropped,
        },
        PlanOp::Link {
            task: 2,
            link: Some("#241".into()),
        },
        PlanOp::Attach {
            task: 2,
            attachment: Attachment::Subagent("a7f3".into()),
        },
    ];
    for op in ops {
        plan.apply(op).unwrap();
    }
    plan
}

fn with_plan() -> orch_protocol::SessionView {
    let mut view = session("webshop", "first");
    view.plan = planned();
    view.subagents.push(SubagentView {
        id: "a7f3".into(),
        agent_type: "Explore".into(),
        description: "find the table".into(),
        tool_count: 3,
        state: SubagentStateView::Working,
        duration_ms: None,
        summary: None,
        failure: None,
        model: None,
        usage: None,
        worktree: None,
    });
    view
}

#[test]
fn the_sidebar_row_shows_done_out_of_tasks_not_dropped() {
    let mut tui = Harness::new();
    tui.sessions(vec![with_plan(), session("webshop", "second")]);
    assert!(tui.sidebar_line_with("first").contains("1/3"));
    assert!(!tui.sidebar_line_with("second").contains("/"));
}

#[test]
fn p_shows_the_selected_sessions_plan_read_only() {
    let mut tui = Harness::new();
    tui.sessions(vec![with_plan()]);
    tui.press(KeyCode::Char('p'));

    let screen = tui.screen();
    assert!(screen.contains(" Plan "), "{screen}");
    assert!(screen.contains("1/3 done"), "{screen}");
    assert!(tui.line_with("Write the migration").contains("ready"));
    assert!(tui.line_with("Wire the CLI").contains("blocked by 2"));
    assert!(screen.contains("#241"), "{screen}");
    assert!(
        screen.contains("Explore: find the table working"),
        "{screen}"
    );

    tui.press(KeyCode::Char('x'));
    assert!(
        tui.screen().contains(" Plan "),
        "other keys don't edit or close"
    );
    tui.press(KeyCode::Esc);
    assert!(!tui.screen().contains(" Plan "));

    tui.command("plan");
    assert!(tui.screen().contains("Design the schema"));
}
