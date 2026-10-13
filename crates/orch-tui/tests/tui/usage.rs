use crossterm::event::KeyCode;
use orch_protocol::{Reply, RepoUsage, Request, UsageReport, UsageTotalsView};

use crate::common::*;

fn totals(input: u64, output: u64, cost: f64) -> UsageTotalsView {
    UsageTotalsView {
        input_tokens: input,
        output_tokens: output,
        cost_usd: cost,
    }
}

#[test]
fn usage_shows_estimated_tokens_and_cost_per_repo_and_for_today() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first")]);
    tui.daemon().script_reply(Ok(Reply::Usage(UsageReport {
        per_repo: vec![
            RepoUsage {
                repo: "/home/me/webshop".into(),
                totals: totals(120_000, 8_000, 3.456),
            },
            RepoUsage {
                repo: "/home/me/dotfiles".into(),
                totals: totals(1_000, 200, 0.02),
            },
        ],
        today: vec![RepoUsage {
            repo: "/home/me/webshop".into(),
            totals: totals(20_000, 1_000, 0.5),
        }],
        total: totals(121_000, 8_200, 3.476),
        estimated: true,
    })));
    tui.command("usage");

    assert_eq!(tui.daemon().requests().pop(), Some(Request::Usage));
    let screen = tui.screen();
    assert!(screen.contains("estimate"), "{screen}");
    let webshop = tui
        .lines()
        .into_iter()
        .find(|line| line.contains("webshop") && line.contains(" in "))
        .expect("a usage row for webshop");
    assert!(webshop.contains("120.0k in"), "{webshop}");
    assert!(webshop.contains("8.0k out"), "{webshop}");
    assert!(webshop.contains("$3.46"), "{webshop}");
    assert!(screen.contains("Today"), "{screen}");
    assert!(tui.line_with("Total").contains("$3.48"));

    tui.press(KeyCode::Esc);
    assert!(!tui.screen().contains("Today"));
}

#[test]
fn usage_is_always_labelled_as_an_estimate() {
    let mut tui = Harness::new();
    tui.sessions(vec![session("webshop", "first")]);
    tui.daemon().script_reply(Ok(Reply::Usage(UsageReport {
        estimated: false,
        ..UsageReport::default()
    })));
    tui.command("usage");
    assert!(tui.screen().contains("(estimates)"));
}
