use orch_holder::{FromHolder, read_frame, write_frame};

#[test]
fn a_holder_message_from_a_newer_holder_is_read_as_unknown_and_the_stream_stays_in_sync() {
    let mut stream = Vec::new();
    let future = serde_json::json!({ "type": "from_the_future", "detail": 1 });
    write_frame(&mut stream, &future).unwrap();
    write_frame(&mut stream, &FromHolder::Superseded).unwrap();

    let mut reader = &stream[..];
    let first: Option<FromHolder> = read_frame(&mut reader).unwrap();
    let second: Option<FromHolder> = read_frame(&mut reader).unwrap();

    assert_eq!(first, Some(FromHolder::Unknown));
    assert_eq!(second, Some(FromHolder::Superseded));
}
