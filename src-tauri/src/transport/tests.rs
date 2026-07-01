use crate::identity::mailbox;
use crate::identity::session_file;
use crate::identity::session_file::SessionFile;
use crate::routing::{send, Message, SendRequest};
use crate::storage::Storage;
use crate::transport::sse::events_stream_raw;
use crate::transport::wake::{SseEvent, SubscriberRegistry};
use std::pin::Pin;
use tempfile::TempDir;
use tokio_stream::StreamExt;

fn dummy_message() -> Message {
    Message {
        id: "msg-1".to_string(),
        to_address: "addr".to_string(),
        to_name: "addr-name".to_string(),
        from_address: "from".to_string(),
        from_name: "from-name".to_string(),
        body: "hi".to_string(),
        content_type: "text".to_string(),
        reply_to_id: None,
        metadata: "{}".to_string(),
        event_id: 1,
        status: "pending".to_string(),
        created_at: 0.0,
    }
}

#[tokio::test]
async fn registry_notifies_subscriber() {
    let registry = SubscriberRegistry::new();
    let mut rx = registry.subscribe("addr");
    registry.notify(
        "addr",
        SseEvent {
            message: dummy_message(),
        },
    );
    let evt = rx.recv().await.unwrap();
    assert_eq!(evt.message.body, "hi");
}

#[tokio::test]
async fn sse_replays_past_events() {
    let tmp = TempDir::new().unwrap();
    let dot = tmp.path().join(".agtalk");
    let storage = Storage::open_in_memory().unwrap();
    let a = mailbox::create(&storage, "nora", "", "").unwrap();
    let b = mailbox::create(&storage, "quinn", "", "").unwrap();
    session_file::write(
        &dot,
        "nora",
        &SessionFile {
            address: a.clone(),
            name: "nora".to_string(),
            workspace: "".to_string(),
            intro: "".to_string(),
            created_at: "2026-07-01T00:00:00Z".to_string(),
        },
    )
    .unwrap();

    send::send(
        &storage,
        SendRequest {
            to: &a,
            to_name: "nora",
            from: &b,
            from_name: "quinn",
            body: "hello",
            content_type: "text",
            reply_to_id: None,
            metadata: "{}",
            more_coming: false,
        },
    )
    .unwrap();

    let registry = SubscriberRegistry::new();
    let rx = registry.subscribe(&a);
    let mut stream = Pin::from(Box::from(events_stream_raw(
        storage,
        a.clone(),
        Some(0),
        rx,
    )));
    let evt = stream.next().await.unwrap();
    assert_eq!(evt.message.body, "hello");
    assert_eq!(evt.message.event_id, 1);
}

#[tokio::test]
async fn sse_realtime_event() {
    let tmp = TempDir::new().unwrap();
    let dot = tmp.path().join(".agtalk");
    let storage = Storage::open_in_memory().unwrap();
    let a = mailbox::create(&storage, "nora", "", "").unwrap();
    session_file::write(
        &dot,
        "nora",
        &SessionFile {
            address: a.clone(),
            name: "nora".to_string(),
            workspace: "".to_string(),
            intro: "".to_string(),
            created_at: "2026-07-01T00:00:00Z".to_string(),
        },
    )
    .unwrap();

    let registry = SubscriberRegistry::new();
    let rx = registry.subscribe(&a);
    let state_storage = storage.clone();
    let mut stream = Pin::from(Box::from(events_stream_raw(
        state_storage,
        a.clone(),
        None,
        rx,
    )));

    // 在 stream 开始等待后实时投递一条消息
    let b = mailbox::create(&storage, "quinn", "", "").unwrap();
    let notify_registry = registry.clone();
    let send_storage = storage.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let msg = send::send(
            &send_storage,
            SendRequest {
                to: &a,
                to_name: "nora",
                from: &b,
                from_name: "quinn",
                body: "realtime",
                content_type: "text",
                reply_to_id: None,
                metadata: "{}",
                more_coming: false,
            },
        )
        .unwrap();
        notify_registry.notify(&a, SseEvent { message: msg });
    });

    let evt = stream.next().await.unwrap();
    assert_eq!(evt.message.body, "realtime");
    assert_eq!(evt.message.event_id, 1);
}
