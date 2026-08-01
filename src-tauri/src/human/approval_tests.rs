use super::*;
use crate::config::HumanConfig;
use crate::identity::mailbox::{create, ensure_human};
use crate::routing::lookup;
use crate::routing::{send::send, SendRequest};

struct Fixture {
    storage: Storage,
    human_addr: String,
    agent_addr: String,
}

fn setup() -> Fixture {
    let storage = Storage::open_in_memory().unwrap();
    let cfg = HumanConfig::default();
    let human_addr = ensure_human(&storage, &cfg).unwrap();
    let agent_addr = create(&storage, "agent", "", "").unwrap();
    Fixture {
        storage,
        human_addr,
        agent_addr,
    }
}

fn send_msg(fx: &Fixture, content_type: &str, metadata: &str) -> Message {
    send(
        &fx.storage,
        SendRequest {
            to: &fx.human_addr,
            to_name: "human",
            from: &fx.agent_addr,
            from_name: "agent",
            body: "question",
            content_type,
            reply_to_id: None,
            subject: None,
            metadata,
            more_coming: false,
        },
    )
    .unwrap()
}

fn req<'a>(msg: &'a Message, body: &'a str, choices: &'a [&'a str]) -> HumanReplyRequest<'a> {
    HumanReplyRequest {
        message_id: &msg.id,
        body,
        choices,
        surface: "popup",
        external_event_id: None,
    }
}

#[test]
fn text_message_allows_multiple_replies() {
    let fx = setup();
    let msg = send_msg(&fx, "text", "{}");

    let first = reply(&fx.storage, req(&msg, "第一条", &[])).unwrap();
    assert_eq!(first.reply.to_address, fx.agent_addr);
    assert_eq!(first.reply.reply_to_id.as_deref(), Some(msg.id.as_str()));
    assert_eq!(first.reply.content_type, "text");
    assert!(!first.resolved);
    // 原消息 pending → read
    let change = first.original_status_change.unwrap();
    assert_eq!(change.old_status, "pending");
    assert_eq!(change.new_status, "read");

    // 再次回复允许，且不再产生状态变化
    let second = reply(&fx.storage, req(&msg, "第二条", &[])).unwrap();
    assert!(second.original_status_change.is_none());
    assert_ne!(first.reply.id, second.reply.id);
}

#[test]
fn approval_first_valid_reply_wins_and_marks_done() {
    let fx = setup();
    let meta = r#"{"choices":["yes","no"],"select_only":false}"#;
    let msg = send_msg(&fx, "approval_request", meta);

    let out = reply(&fx.storage, req(&msg, "同意", &["yes"])).unwrap();
    assert!(out.resolved);
    assert_eq!(out.reply.content_type, "approval_response");
    assert_eq!(out.reply.metadata, r#"{"choice":"yes"}"#);
    let change = out.original_status_change.unwrap();
    assert_eq!(change.new_status, "done");

    // resolution 落库
    let conn = fx.storage.conn();
    let (resolved_by, choice): (String, String) = conn
        .query_row(
            "SELECT resolved_by, selected_choice FROM approval_resolutions \
             WHERE request_message_id = ?1",
            [&msg.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(resolved_by, "popup");
    assert_eq!(choice, "yes");
}

#[test]
fn approval_multi_select_joins_choices() {
    let fx = setup();
    let meta = r#"{"choices":["a","b","c"],"select_only":false}"#;
    let msg = send_msg(&fx, "approval_request", meta);

    let out = reply(&fx.storage, req(&msg, "选两个", &["a", "b"])).unwrap();
    assert!(out.resolved);
    assert_eq!(out.reply.metadata, r#"{"choice":"a、b"}"#);
    let conn = fx.storage.conn();
    let selected: String = conn
        .query_row(
            "SELECT selected_choice FROM approval_resolutions WHERE request_message_id = ?1",
            [&msg.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(selected, "a、b");
}

#[test]
fn approval_single_rejects_multiple_choices() {
    let fx = setup();
    let meta = r#"{"choices":["a","b"],"single":true,"select_only":false}"#;
    let msg = send_msg(&fx, "approval_request", meta);

    match reply(&fx.storage, req(&msg, "都要", &["a", "b"])) {
        Err(HumanError::SingleChoiceOnly(2)) => {}
        other => panic!("expected SingleChoiceOnly, got {:?}", other.is_ok()),
    }
    // 单选合法
    let out = reply(&fx.storage, req(&msg, "选 a", &["a"])).unwrap();
    assert!(out.resolved);
}

#[test]
fn approval_multi_select_rejects_any_invalid_choice() {
    let fx = setup();
    let meta = r#"{"choices":["a","b"],"select_only":false}"#;
    let msg = send_msg(&fx, "approval_request", meta);

    match reply(&fx.storage, req(&msg, "夹带", &["a", "x"])) {
        Err(HumanError::InvalidChoice(c)) => assert_eq!(c, "x"),
        other => panic!("expected InvalidChoice, got {:?}", other.is_ok()),
    }
}

#[test]
fn approval_reply_empty_body_fills_selection_summary() {
    let fx = setup();
    let meta = r#"{"choices":["a","b"],"select_only":false}"#;
    let msg = send_msg(&fx, "approval_request", meta);

    // popup 路径：只传 choices 不传补充文本 → 正文补「选择：X」，agent 可读
    let out = reply(&fx.storage, req(&msg, "", &["a", "b"])).unwrap();
    assert_eq!(out.reply.body, "选择：a、b");
    assert_eq!(out.reply.metadata, r#"{"choice":"a、b"}"#);
}

#[test]
fn approval_second_reply_returns_already_resolved() {
    let fx = setup();
    let meta = r#"{"choices":["yes","no"],"select_only":false}"#;
    let msg = send_msg(&fx, "approval_request", meta);

    let first = reply(&fx.storage, req(&msg, "同意", &["yes"])).unwrap();
    match reply(&fx.storage, req(&msg, "反对", &["no"])) {
        Err(HumanError::AlreadyResolved {
            resolved_by,
            response_id,
        }) => {
            assert_eq!(resolved_by, "popup");
            assert_eq!(response_id, first.reply.id);
        }
        other => panic!("expected AlreadyResolved, got {:?}", other.is_ok()),
    }
}

#[test]
fn select_only_rejects_free_text() {
    let fx = setup();
    let meta = r#"{"choices":["a","b"],"select_only":true}"#;
    let msg = send_msg(&fx, "approval_request", meta);

    match reply(&fx.storage, req(&msg, "随便说说", &[])) {
        Err(HumanError::SelectOnlyRequiresChoice) => {}
        other => panic!("expected SelectOnlyRequiresChoice, got {:?}", other.is_ok()),
    }
}

#[test]
fn invalid_choice_rejected() {
    let fx = setup();
    let meta = r#"{"choices":["a","b"],"select_only":false}"#;
    let msg = send_msg(&fx, "approval_request", meta);

    match reply(&fx.storage, req(&msg, "", &["c"])) {
        Err(HumanError::InvalidChoice(c)) => assert_eq!(c, "c"),
        other => panic!("expected InvalidChoice, got {:?}", other.is_ok()),
    }
}

#[test]
fn reply_to_non_human_message_rejected() {
    let fx = setup();
    // agent → agent 的消息，human 无权回复
    let other_agent = create(&fx.storage, "agent2", "", "").unwrap();
    let msg = send(
        &fx.storage,
        SendRequest {
            to: &other_agent,
            to_name: "agent2",
            from: &fx.agent_addr,
            from_name: "agent",
            body: "not for human",
            content_type: "text",
            reply_to_id: None,
            subject: None,
            metadata: "{}",
            more_coming: false,
        },
    )
    .unwrap();

    match reply(&fx.storage, req(&msg, "hack", &[])) {
        Err(HumanError::MessageNotFound(_)) => {}
        other => panic!("expected MessageNotFound, got {:?}", other.is_ok()),
    }
}

#[test]
fn duplicate_external_event_replays_original_reply() {
    let fx = setup();
    let msg = send_msg(&fx, "text", "{}");

    let r1 = HumanReplyRequest {
        external_event_id: Some("evt-1"),
        ..req(&msg, "第一条", &[])
    };
    let first = reply(&fx.storage, r1).unwrap();
    assert!(!first.deduplicated);

    // 重复事件：回放首次结果，不重复创建回复
    let r2 = HumanReplyRequest {
        external_event_id: Some("evt-1"),
        ..req(&msg, "重复", &[])
    };
    let second = reply(&fx.storage, r2).unwrap();
    assert!(second.deduplicated);
    assert_eq!(second.reply.id, first.reply.id);

    // agent inbox 里只有一条回复
    let inbox = crate::routing::inbox::inbox(&fx.storage, &fx.agent_addr, false).unwrap();
    let replies: Vec<_> = inbox
        .iter()
        .filter(|m| m.reply_to_id.as_deref() == Some(msg.id.as_str()))
        .collect();
    assert_eq!(replies.len(), 1);
}

#[test]
fn inconclusive_receipt_returns_explicit_error() {
    let fx = setup();
    let msg = send_msg(&fx, "text", "{}");

    // 历史占位数据：receipt 无结果 id，无法确定原动作是否已落库
    {
        let conn = fx.storage.conn();
        delivery::record_receipt(&conn, "popup", "evt-crash", None, "reply").unwrap();
    }

    let r = HumanReplyRequest {
        external_event_id: Some("evt-crash"),
        ..req(&msg, "重试", &[])
    };
    match reply(&fx.storage, r) {
        Err(HumanError::ReceiptInconclusive { surface, event }) => {
            assert_eq!(surface, "popup");
            assert_eq!(event, "evt-crash");
        }
        other => panic!("expected ReceiptInconclusive, got {:?}", other.is_ok()),
    }
}

#[test]
fn failed_reply_rolls_back_receipt_and_allows_retry() {
    let fx = setup();
    let msg = send_msg(&fx, "text", "{}");

    // 第一次执行在事务内失败（原消息不存在）：receipt 随事务回滚
    let bad = HumanReplyRequest {
        message_id: "no-such-msg",
        external_event_id: Some("evt-retry"),
        ..req(&msg, "失败", &[])
    };
    assert!(matches!(
        reply(&fx.storage, bad),
        Err(HumanError::MessageNotFound(_))
    ));
    let count: i64 = fx
        .storage
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM human_action_receipts WHERE external_event_id = 'evt-retry'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);

    // 同一事件修正后重试成功
    let good = HumanReplyRequest {
        external_event_id: Some("evt-retry"),
        ..req(&msg, "重试", &[])
    };
    let out = reply(&fx.storage, good).unwrap();
    assert!(!out.deduplicated);
}

#[test]
fn reply_allocates_event_id_on_receiver() {
    let fx = setup();
    let msg = send_msg(&fx, "text", "{}");
    let out = reply(&fx.storage, req(&msg, "hi", &[])).unwrap();
    // 回复消息落在 agent 的 event 序列上，可用 agent inbox 查到
    let inbox = crate::routing::inbox::inbox(&fx.storage, &fx.agent_addr, false).unwrap();
    assert!(inbox.iter().any(|m| m.id == out.reply.id));
    // 短 ID 可解析
    let resolved = lookup::resolve_id(&fx.storage, &fx.agent_addr, &out.reply.id[..8]).unwrap();
    assert_eq!(resolved, out.reply.id);
}

#[test]
fn cancel_creates_cancelled_reply_and_marks_done() {
    let fx = setup();
    let msg = send_msg(&fx, "text", "{}");

    let (reply_msg, dedup) =
        cancel_with_receipt(&fx.storage, &msg.id, "feishu", Some("evt-c1")).unwrap();
    assert!(!dedup);
    assert_eq!(reply_msg.body, "（已取消）");
    assert_eq!(reply_msg.to_address, fx.agent_addr);
    assert_eq!(reply_msg.reply_to_id.as_deref(), Some(msg.id.as_str()));
    assert_eq!(reply_msg.metadata, r#"{"cancelled":true}"#);

    // 原消息置 done（conn guard 必须在后续 storage 调用前释放，避免死锁）
    {
        let conn = fx.storage.conn();
        let status: String = conn
            .query_row(
                "SELECT status FROM messages WHERE id = ?1",
                [&msg.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "done");
    }

    // 重复事件：回放首次结果，不重复创建
    let (replayed, dedup) =
        cancel_with_receipt(&fx.storage, &msg.id, "feishu", Some("evt-c1")).unwrap();
    assert!(dedup);
    assert_eq!(replayed.id, reply_msg.id);
    let inbox = crate::routing::inbox::inbox(&fx.storage, &fx.agent_addr, false).unwrap();
    let cancels: Vec<_> = inbox
        .iter()
        .filter(|m| m.reply_to_id.as_deref() == Some(msg.id.as_str()))
        .collect();
    assert_eq!(cancels.len(), 1);
}

#[test]
fn cancel_approval_writes_resolution_and_blocks_later_choice() {
    let fx = setup();
    let meta = r#"{"choices":["yes","no"],"select_only":true}"#;
    let msg = send_msg(&fx, "approval_request", meta);

    let (reply_msg, _) =
        cancel_with_receipt(&fx.storage, &msg.id, "feishu", Some("evt-c2")).unwrap();
    {
        let conn = fx.storage.conn();
        let (resolution, selected): (String, Option<String>) = conn
            .query_row(
                "SELECT resolution, selected_choice FROM approval_resolutions \
                 WHERE request_message_id = ?1",
                [&msg.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(resolution, "cancelled");
        assert_eq!(selected, None);
    }

    // 取消后旧卡再点选项：被仲裁拒绝
    match reply(&fx.storage, req(&msg, "同意", &["yes"])) {
        Err(HumanError::AlreadyResolved {
            resolved_by,
            response_id,
        }) => {
            assert_eq!(resolved_by, "feishu");
            assert_eq!(response_id, reply_msg.id);
        }
        other => panic!("expected AlreadyResolved, got {:?}", other.is_ok()),
    }
}

#[test]
fn cancel_after_resolved_returns_already_resolved() {
    let fx = setup();
    let meta = r#"{"choices":["yes","no"],"select_only":false}"#;
    let msg = send_msg(&fx, "approval_request", meta);

    let first = reply(&fx.storage, req(&msg, "同意", &["yes"])).unwrap();
    match cancel_with_receipt(&fx.storage, &msg.id, "feishu", Some("evt-c3")) {
        Err(HumanError::AlreadyResolved {
            resolved_by,
            response_id,
        }) => {
            assert_eq!(resolved_by, "popup");
            assert_eq!(response_id, first.reply.id);
        }
        other => panic!("expected AlreadyResolved, got {:?}", other.is_ok()),
    }
}

#[test]
fn cancel_to_non_human_message_rejected() {
    let fx = setup();
    let other_agent = create(&fx.storage, "agent2", "", "").unwrap();
    let msg = send(
        &fx.storage,
        SendRequest {
            to: &other_agent,
            to_name: "agent2",
            from: &fx.agent_addr,
            from_name: "agent",
            body: "not for human",
            content_type: "text",
            reply_to_id: None,
            subject: None,
            metadata: "{}",
            more_coming: false,
        },
    )
    .unwrap();
    match cancel_with_receipt(&fx.storage, &msg.id, "feishu", None) {
        Err(HumanError::MessageNotFound(_)) => {}
        other => panic!("expected MessageNotFound, got {:?}", other.is_ok()),
    }
}
