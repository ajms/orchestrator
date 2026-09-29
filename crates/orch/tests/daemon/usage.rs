use orch_core::SessionId;
use orch_protocol::{CreateSession, Reply, Request, UsageReport, UsageTotalsView};

use crate::common::*;

fn statusline(conversation: &str, input: u64, output: u64, cost: f64) -> String {
    format!(
        r#"{{"session_id":"{conversation}","model":{{"display_name":"Opus"}},"context_window":{{"total_input_tokens":{input},"total_output_tokens":{output},"used_percentage":12}},"cost":{{"total_cost_usd":{cost}}}}}"#
    )
}

async fn tap(
    client: &mut TestClient,
    pane: &mut PaneView,
    id: &SessionId,
    (conversation, input, output, cost): (&str, u64, u64, f64),
) {
    pane.type_line(&format!(
        "tap {}",
        statusline(conversation, input, output, cost)
    ))
    .await;
    client
        .until(id, "the sample", |view| view.cost_usd == Some(cost))
        .await;
}

async fn usage(client: &mut TestClient) -> UsageReport {
    match client.request(Request::Usage).await {
        Ok(Reply::Usage(report)) => report,
        other => panic!("usage failed: {other:?}"),
    }
}

fn assert_totals(actual: UsageTotalsView, input: u64, output: u64, cost: f64) {
    assert_eq!((actual.input_tokens, actual.output_tokens), (input, output));
    assert!((actual.cost_usd - cost).abs() < 1e-9, "{actual:?}");
}

#[tokio::test]
async fn usage_is_totalled_per_repo_and_for_today_across_conversations_and_agent_restarts() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (app, mut app_pane) = idle_session(&env, &mut client, "App work").await;
    tap(&mut client, &mut app_pane, &app, ("conv-1", 100, 10, 0.5)).await;
    tap(&mut client, &mut app_pane, &app, ("conv-1", 300, 30, 1.5)).await;
    tap(&mut client, &mut app_pane, &app, ("conv-2", 50, 5, 0.25)).await;
    tap(&mut client, &mut app_pane, &app, ("conv-2", 20, 2, 0.125)).await;

    let lib_repo = env.repo("lib");
    let lib = client
        .create(CreateSession::new(&lib_repo, "Lib work"))
        .await;
    client
        .until(&lib, "running", |view| view.agent.is_some())
        .await;
    let mut lib_pane = env.pane(&lib, PANE).await;
    tap(&mut client, &mut lib_pane, &lib, ("conv-9", 1000, 100, 2.0)).await;

    let report = usage(&mut client).await;
    assert!(report.estimated);
    let app_repo = env.path("repos/app");
    for per_repo in [&report.per_repo, &report.today] {
        let repos: Vec<_> = per_repo.iter().map(|entry| entry.repo.clone()).collect();
        assert_eq!(repos, [app_repo.clone(), lib_repo.clone()]);
        assert_totals(per_repo[0].totals, 370, 37, 1.875);
        assert_totals(per_repo[1].totals, 1000, 100, 2.0);
    }
    assert_totals(report.total, 1370, 137, 3.875);
}

fn limits(five_hour: Option<f64>, seven_day: Option<f64>) -> RateLimits {
    RateLimits {
        five_hour,
        seven_day,
    }
}

fn limited(five_hour: f64, seven_day: Option<f64>) -> String {
    let seven_day = seven_day.map_or(String::new(), |used| {
        format!(r#","seven_day":{{"used_percentage":{used},"resets_at":1760000000}}"#)
    });
    format!(
        r#"tap {{"session_id":"conv-1","rate_limits":{{"five_hour":{{"used_percentage":{five_hour},"resets_at":1750000000}}{seven_day}}}}}"#
    )
}

#[tokio::test]
async fn rate_limits_are_broadcast_on_change_and_to_clients_as_they_connect() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let (_id, mut pane) = idle_session(&env, &mut client, "Busy").await;

    pane.type_line(&limited(83.0, None)).await;
    client
        .until_received("rate limits", |client| !client.rate_limits.is_empty())
        .await;
    assert_eq!(client.rate_limits, [limits(Some(83.0), None)]);

    let mut late = env.client().await;
    late.until_received("rate limits on connect", |client| {
        !client.rate_limits.is_empty()
    })
    .await;
    assert_eq!(late.rate_limits, [limits(Some(83.0), None)]);

    pane.type_line(&limited(83.0, Some(40.0))).await;
    pane.type_line(&limited(83.0, Some(40.0))).await;
    pane.type_line(&limited(96.0, Some(40.0))).await;
    client
        .until_received("the latest rate limits", |client| {
            client.rate_limits.last() == Some(&limits(Some(96.0), Some(40.0)))
        })
        .await;
    assert_eq!(
        client.rate_limits,
        [
            limits(Some(83.0), None),
            limits(Some(83.0), Some(40.0)),
            limits(Some(96.0), Some(40.0))
        ]
    );
}
