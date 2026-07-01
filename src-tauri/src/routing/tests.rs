use crate::identity::mailbox;
use crate::routing::{inbox, lookup, reply, send, SendRequest};
use crate::storage::Storage;

fn setup() -> (Storage, String, String) {
    let storage = Storage::open_in_memory().unwrap();
    let a = mailbox::create(&storage, "nora", "前端", "projA").unwrap();
    let b = mailbox::create(&storage, "quinn", "设计", "projA").unwrap();
    (storage, a, b)
}

fn req<'a>(
    to: &'a str,
    to_name: &'a str,
    from: &'a str,
    from_name: &'a str,
    body: &'a str,
) -> SendRequest<'a> {
    SendRequest {
        to,
        to_name,
        from,
        from_name,
        body,
        content_type: "text",
        reply_to_id: None,
        metadata: "{}",
        more_coming: false,
    }
}

#[test]
fn send_allocates_event_id() {
    let (storage, a, b) = setup();
    let msg = send::send(&storage, req(&b, "quinn", &a, "nora", "hello")).unwrap();
    assert_eq!(msg.event_id, 1);
    assert_eq!(msg.status, "pending");
    assert_eq!(msg.from_name, "nora");
    assert_eq!(msg.to_name, "quinn");
}

#[test]
fn send_fails_to_unknown_address() {
    let (storage, a, _b) = setup();
    let result = send::send(
        &storage,
        SendRequest {
            to: "00000000-0000-0000-0000-000000000000",
            to_name: "unknown",
            from: &a,
            from_name: "nora",
            body: "x",
            content_type: "text",
            reply_to_id: None,
            metadata: "{}",
            more_coming: false,
        },
    );
    assert!(result.is_err());
}

#[test]
fn inbox_filters_done() {
    let (storage, a, b) = setup();
    send::send(&storage, req(&b, "quinn", &a, "nora", "m1")).unwrap();
    send::send(&storage, req(&b, "quinn", &a, "nora", "m2")).unwrap();
    inbox::mark_delivered(
        &storage,
        &send::send(&storage, req(&b, "quinn", &a, "nora", "m3"))
            .unwrap()
            .id,
    )
    .unwrap();
    inbox::mark_read(
        &storage,
        &send::send(&storage, req(&b, "quinn", &a, "nora", "m4"))
            .unwrap()
            .id,
    )
    .unwrap();

    let open = inbox::inbox(&storage, &b, false).unwrap();
    assert_eq!(open.len(), 4);

    // 将一条标记为 done
    let id = &open[0].id;
    {
        let conn = storage.conn();
        conn.execute("UPDATE messages SET status = 'done' WHERE id = ?1", [id])
            .unwrap();
    }
    let open = inbox::inbox(&storage, &b, false).unwrap();
    assert_eq!(open.len(), 3);

    let all = inbox::inbox(&storage, &b, true).unwrap();
    assert_eq!(all.len(), 4);
}

#[test]
fn reply_to_message() {
    let (storage, a, b) = setup();
    let original = send::send(
        &storage,
        SendRequest {
            to: &b,
            to_name: "quinn",
            from: &a,
            from_name: "nora",
            body: "审批？",
            content_type: "approval_request",
            reply_to_id: None,
            metadata: r#"{"choices":["approve","reject"]}"#,
            more_coming: false,
        },
    )
    .unwrap();
    let resp = reply::reply(
        &storage,
        &original.id,
        &b,
        "quinn",
        "approve",
        Some("approve"),
    )
    .unwrap();
    assert_eq!(resp.content_type, "approval_response");
    assert_eq!(resp.reply_to_id, Some(original.id.clone()));
    assert_eq!(resp.to_address, a);

    let original_after = lookup::detail(&storage, &original.id).unwrap().unwrap();
    assert_eq!(original_after.status, "done");
}

#[test]
fn lookup_by_name() {
    let (storage, _a, _b) = setup();
    mailbox::create(&storage, "nora", "后端", "projB").unwrap();
    let all = lookup::lookup(&storage, None).unwrap();
    assert_eq!(all.len(), 3);
    let noras = lookup::lookup(&storage, Some("nora")).unwrap();
    assert_eq!(noras.len(), 2);
}
