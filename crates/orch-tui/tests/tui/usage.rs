use crossterm::event::KeyCode;
use orch_protocol::{AgentTotals, Reply, RepoUsage, Request, UsageReport, UsageTotalsView};

use crate::common::*;

fn totals(input: u64, output: u64, cost: Option<f64>) -> UsageTotalsView {
    UsageTotalsView {
        input_tokens: input,
        output_tokens: output,
        cost_usd: cost,
    }
}

fn repo(path: &str, agent: &str, totals: UsageTotalsView) -> RepoUsage {
    RepoUsage {
        repo: path.into(),
        agent: agent.into(),
        totals,
    }
}

fn agent(agent: &str, totals: UsageTotalsView) -> AgentTotals {
    AgentTotals {
        agent: agent.into(),
        totals,
    }
}

#[test]
fn usage_shows_tokens_and_cost_per_repo_and_for_today() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first")]);
    tui.daemon().script_reply(Ok(Reply::Usage(UsageReport {
        per_repo: vec![
            repo(
                "/home/me/webshop",
                "claude",
                totals(120_000, 8_000, Some(3.456)),
            ),
            repo(
                "/home/me/dotfiles",
                "claude",
                totals(1_000, 200, Some(0.02)),
            ),
        ],
        today: vec![repo(
            "/home/me/webshop",
            "claude",
            totals(20_000, 1_000, Some(0.5)),
        )],
        per_agent: vec![agent("claude", totals(121_000, 8_200, Some(3.476)))],
        estimated: true,
    })));
    tui.command("usage");

    assert_eq!(tui.daemon().requests().pop(), Some(Request::Usage));
    let screen = tui.screen();
    let webshop = tui
        .lines()
        .into_iter()
        .find(|line| line.contains("webshop") && line.contains(" in "))
        .expect("a usage row for webshop");
    assert!(webshop.contains("120.0k in"), "{webshop}");
    assert!(webshop.contains("8.0k out"), "{webshop}");
    assert!(webshop.contains("$3.46"), "{webshop}");
    assert!(screen.contains("Today"), "{screen}");
    assert!(tui.screen().contains("Total"));
    let total = tui
        .lines()
        .into_iter()
        .rfind(|line| line.contains("claude") && line.contains(" in "))
        .expect("a total row for claude");
    assert!(total.contains("$3.48"), "{total}");

    tui.press(KeyCode::Esc);
    assert!(!tui.screen().contains("Today"));
}

#[test]
fn usage_is_split_per_agent_and_an_unknown_cost_is_never_shown_as_zero() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first")]);
    tui.daemon().script_reply(Ok(Reply::Usage(UsageReport {
        per_repo: vec![
            repo(
                "/home/me/webshop",
                "antigravity",
                totals(40_000, 2_000, None),
            ),
            repo("/home/me/webshop", "claude", totals(9_000, 900, Some(1.25))),
        ],
        today: Vec::new(),
        per_agent: vec![
            agent("antigravity", totals(40_000, 2_000, None)),
            agent("claude", totals(9_000, 900, Some(1.25))),
        ],
        estimated: true,
    })));
    tui.command("usage");

    let mut rows = |agent: &str| -> Vec<String> {
        tui.lines()
            .into_iter()
            .filter(|line| line.contains(agent) && line.contains(" in "))
            .collect()
    };
    let agy = rows("antigravity");
    assert_eq!(agy.len(), 2, "{agy:?}");
    for row in &agy {
        assert!(row.contains("40.0k in"), "{row}");
        assert!(row.contains("cost unknown"), "{row}");
        assert!(!row.contains('$'), "{row}");
    }
    let claude = rows("claude");
    assert_eq!(claude.len(), 2, "{claude:?}");
    assert!(claude.iter().all(|row| row.contains("$1.25")), "{claude:?}");
}

#[test]
fn usage_is_always_labelled_as_approximate_agent_reported_totals() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first")]);
    tui.daemon().script_reply(Ok(Reply::Usage(UsageReport {
        estimated: false,
        ..UsageReport::default()
    })));
    tui.command("usage");
    let screen = tui.screen();
    assert!(
        screen.contains(":usage (approximate, as each Agent reports it)"),
        "{screen}"
    );
    assert!(!screen.contains("estimate"), "{screen}");
}
