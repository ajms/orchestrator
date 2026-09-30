use orch_holder::{FromHolder, HolderClient};

use crate::common::*;

async fn next_copy(client: &mut HolderClient) -> String {
    next_matching(client, "copy", |message| match message {
        FromHolder::Clipboard { text } => Some(text),
        _ => None,
    })
    .await
}

#[tokio::test]
async fn an_agent_osc_52_copy_is_relayed_decoded_to_the_attached_daemon() {
    let sandbox = Sandbox::new();
    let held = sandbox.hold("s1", "");
    let (mut client, _) = held.attach().await;

    type_line(&mut client, "osc52 copied by the agent").await;

    assert_eq!(next_copy(&mut client).await, "copied by the agent");
}
