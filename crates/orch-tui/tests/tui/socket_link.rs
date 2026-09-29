use std::time::Duration;

use orch_holder::{read_frame_async, write_frame_async};
use orch_protocol::{FromDaemon, OpenPane, PROTOCOL_VERSION, Reply, Request, Size, ToDaemon};
use orch_tui::{DaemonLink, Event, PaneId, RequestId, SocketLink};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

use crate::common::id;

const PANE: Size = Size { rows: 10, cols: 40 };

async fn next(rx: &mut mpsc::UnboundedReceiver<Event>) -> Event {
    tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("an event in time")
        .expect("the link is open")
}

#[tokio::test]
async fn the_socket_link_speaks_the_daemon_protocol_on_control_and_pane_connections() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (control, _) = listener.accept().await.unwrap();
        let (mut control_in, mut control_out) = control.into_split();
        let hello: ToDaemon = read_frame_async(&mut control_in).await.unwrap().unwrap();
        assert_eq!(
            hello,
            ToDaemon::Hello {
                version: PROTOCOL_VERSION,
                pane: None
            }
        );
        let welcome = FromDaemon::Welcome {
            version: PROTOCOL_VERSION,
        };
        write_frame_async(&mut control_out, &welcome).await.unwrap();
        write_frame_async(&mut control_out, &FromDaemon::Sessions { sessions: vec![] })
            .await
            .unwrap();
        let request: ToDaemon = read_frame_async(&mut control_in).await.unwrap().unwrap();
        let ToDaemon::Request { id, request } = request else {
            panic!("expected a request, got {request:?}");
        };
        assert_eq!(id, 5);
        assert_eq!(
            request,
            Request::Resume {
                session: crate::common::id("first")
            }
        );
        let response = FromDaemon::Response {
            id,
            result: Ok(Reply::Done),
        };
        write_frame_async(&mut control_out, &response)
            .await
            .unwrap();

        let (pane, _) = listener.accept().await.unwrap();
        let (mut pane_in, mut pane_out) = pane.into_split();
        let hello: ToDaemon = read_frame_async(&mut pane_in).await.unwrap().unwrap();
        assert_eq!(
            hello,
            ToDaemon::Hello {
                version: PROTOCOL_VERSION,
                pane: Some(OpenPane {
                    session: crate::common::id("first"),
                    size: PANE
                }),
            }
        );
        write_frame_async(&mut pane_out, &welcome).await.unwrap();
        let output = FromDaemon::Output {
            bytes: b"hi".to_vec(),
        };
        write_frame_async(&mut pane_out, &output).await.unwrap();
        let input: ToDaemon = read_frame_async(&mut pane_in).await.unwrap().unwrap();
        assert_eq!(
            input,
            ToDaemon::Input {
                bytes: b"x".to_vec()
            }
        );
        let paste: ToDaemon = read_frame_async(&mut pane_in).await.unwrap().unwrap();
        assert_eq!(paste, ToDaemon::Paste { text: "p".into() });
        let resize: ToDaemon = read_frame_async(&mut pane_in).await.unwrap().unwrap();
        assert_eq!(resize, ToDaemon::Resize(Size { rows: 11, cols: 41 }));
    });

    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut link = SocketLink::connect(&socket, tx).await.unwrap();
    assert!(matches!(
        next(&mut rx).await,
        Event::Daemon(FromDaemon::Sessions { .. })
    ));

    link.request(
        RequestId(5),
        Request::Resume {
            session: id("first"),
        },
    );
    assert!(matches!(
        next(&mut rx).await,
        Event::Daemon(FromDaemon::Response { id: 5, .. })
    ));

    link.open_pane(PaneId(3), &id("first"), PANE);
    link.input(b"x".to_vec());
    link.paste("p".into());
    link.resize_pane(Size { rows: 11, cols: 41 });
    match next(&mut rx).await {
        Event::Pane { pane, message } => {
            assert_eq!(pane, PaneId(3));
            assert_eq!(
                message,
                FromDaemon::Output {
                    bytes: b"hi".to_vec()
                }
            );
        }
        other => panic!("expected pane output, got {other:?}"),
    }
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn a_version_mismatch_is_reported_on_connect() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (mut input, mut output) = stream.into_split();
        let _: Option<ToDaemon> = read_frame_async(&mut input).await.unwrap();
        let mismatch = FromDaemon::VersionMismatch {
            daemon_version: 0,
            message: "old Daemon".into(),
        };
        write_frame_async(&mut output, &mismatch).await.unwrap();
    });

    let (tx, _rx) = mpsc::unbounded_channel();
    match SocketLink::connect(&socket, tx).await {
        Err(orch_protocol::ConnectError::VersionMismatch { message, .. }) => {
            assert_eq!(message, "old Daemon");
        }
        Err(other) => panic!("expected a version mismatch, got {other}"),
        Ok(_) => panic!("expected a version mismatch"),
    }
}

#[tokio::test]
async fn an_offline_link_tells_the_user_instead_of_dropping_requests_silently() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut link = SocketLink::offline(std::path::Path::new("/nonexistent"), tx);
    link.request(
        RequestId(1),
        Request::Resume {
            session: id("first"),
        },
    );
    match next(&mut rx).await {
        Event::Notice(text) => assert!(text.contains("not connected to the Daemon"), "{text}"),
        other => panic!("expected a notice, got {other:?}"),
    }
}
