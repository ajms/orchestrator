use crate::common::*;

#[tokio::test]
async fn an_agent_copy_reaches_only_the_clients_showing_its_session() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut creator = env.client().await;
    let first = running_session(&env, &mut creator, "First").await;
    let second = running_session(&env, &mut creator, "Second").await;
    let mut watching_first = env.client().await;
    watching_first.view(Some(&first), true).await;
    let mut watching_second = env.client().await;
    watching_second.view(Some(&second), false).await;

    let mut first_pane = env.pane(&first, PANE).await;
    first_pane.type_line("osc52 from the first agent").await;
    watching_first
        .until_received("the first copy", |client| !client.copies.is_empty())
        .await;
    let mut second_pane = env.pane(&second, PANE).await;
    second_pane.type_line("osc52 from the second agent").await;
    watching_second
        .until_received("the second copy", |client| !client.copies.is_empty())
        .await;

    assert_eq!(
        watching_first.copies,
        vec![(first.clone(), "from the first agent".to_string())]
    );
    assert_eq!(
        watching_second.copies,
        vec![(second.clone(), "from the second agent".to_string())]
    );
    creator.drain().await;
    assert!(creator.copies.is_empty(), "{:?}", creator.copies);
}

#[tokio::test]
async fn an_agent_copy_is_dropped_when_no_client_shows_its_session() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Unwatched").await;
    let mut pane = env.pane(&id, PANE).await;
    pane.type_line("osc52 nobody looks").await;
    pane.type_line("print copied unseen").await;
    pane.wait_for_text("copied unseen").await;
    pane.hook(&hook("SessionStart", "")).await;
    client
        .until(&id, "Idle", |view| {
            view.agent == Some(orch_protocol::AgentStateView::Idle)
        })
        .await;

    client.view(Some(&id), true).await;
    pane.type_line("osc52 now shown").await;
    client
        .until_received("the shown copy", |client| !client.copies.is_empty())
        .await;

    assert_eq!(client.copies, vec![(id, "now shown".to_string())]);
}
