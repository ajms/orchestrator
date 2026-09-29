use orch_protocol::{CreateSession, FromDaemon, PhaseView, Size};

use crate::common::*;

#[tokio::test]
async fn a_pane_opens_on_the_current_screen_and_relays_input_and_paste() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Show me").await;

    let mut first = env.pane(&id, PANE).await;
    first.type_line("print typed into the first pane").await;
    first.wait_for_text("typed into the first pane").await;

    let mut second = env.pane(&id, PANE).await;
    second.wait_for_text("typed into the first pane").await;
    second.pane.paste("print pasted text\r").await.unwrap();
    second.wait_for_text("pasted text").await;
    first.wait_for_text("pasted text").await;
}

#[tokio::test]
async fn every_client_sees_every_session_and_its_changes() {
    let env = Env::new();
    let repo = env.repo("app");
    let _daemon = env.start_daemon().await;
    let mut early = env.client().await;
    early.session_list().await;
    let mut creator = env.client().await;
    let id = creator.create(CreateSession::new(&repo, "Shared")).await;

    early
        .until(&id, "Active", |view| view.phase == PhaseView::Active)
        .await;
    let mut late = env.client().await;
    let listed = late.session_list().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, id);
}

#[tokio::test]
async fn a_pane_is_sized_to_the_most_recently_active_client() {
    let env = Env::new();
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = running_session(&env, &mut client, "Resize").await;
    let small = Size { rows: 20, cols: 90 };
    let large = Size {
        rows: 40,
        cols: 120,
    };

    let mut first = env.pane(&id, small).await;
    first
        .wait_for("the small size", |pane| pane.size() == small)
        .await;
    let mut second = env.pane(&id, large).await;
    second
        .wait_for("the large size", |pane| pane.size() == large)
        .await;
    first
        .wait_for("the large size", |pane| pane.size() == large)
        .await;
    second.type_line("size").await;
    second.wait_for_text("40 120").await;

    first.type_line("size").await;
    first
        .wait_for("the small size", |pane| pane.size() == small)
        .await;
    first.wait_for_text("20 90").await;
    second
        .wait_for("the small size", |pane| pane.size() == small)
        .await;

    second
        .pane
        .resize(Size {
            rows: 30,
            cols: 100,
        })
        .await
        .unwrap();
    first
        .wait_for("the resized size", |pane| {
            pane.size()
                == Size {
                    rows: 30,
                    cols: 100,
                }
        })
        .await;
}

#[tokio::test]
async fn a_pane_of_a_session_without_a_holder_is_closed_with_a_reason() {
    let env = Env::new();
    let repo = env.repo("app");
    env.write_config(&format!("[repos.{repo:?}]\nsetup = \"false\"\n"));
    let _daemon = env.start_daemon().await;
    let mut client = env.client().await;
    let id = client.create(CreateSession::new(&repo, "No agent")).await;
    client
        .until(&id, "Setup failed", |view| {
            view.phase == PhaseView::SetupFailed
        })
        .await;
    let mut pane = orch_protocol::Pane::open(&env.socket(), &id, PANE)
        .await
        .unwrap();
    let closed = tokio::time::timeout(WAIT, pane.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(closed, Some(FromDaemon::PaneClosed { .. })),
        "{closed:?}"
    );
}
