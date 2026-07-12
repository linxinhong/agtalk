//! decide 的单元测试（就近原则；从 router.rs 拆出）。

use super::*;
use crate::config::HumanConfig;
use crate::feishu::router::LinkStatus;
use crate::identity::mailbox::{create, ensure_human};
use crate::routing::{send::send, SendRequest};
use serde_json::json;

fn setup() -> (Storage, String) {
    let storage = Storage::open_in_memory().unwrap();
    let human_addr = ensure_human(&storage, &HumanConfig::default()).unwrap();
    (storage, human_addr)
}

fn send_approval(storage: &Storage, human_addr: &str) -> Message {
    let agent = create(storage, "agent", "", "").unwrap();
    send(
        storage,
        SendRequest {
            to: human_addr,
            to_name: "human",
            from: &agent,
            from_name: "agent",
            body: "部署到生产？",
            content_type: "approval_request",
            reply_to_id: None,
            subject: None,
            metadata: &json!({ "choices": ["批准", "拒绝"], "select_only": true }).to_string(),
            more_coming: false,
        },
    )
    .unwrap()
}

fn card_event(msg_id: &str, choice_index: usize, operator: &str) -> Value {
    json!({
        "operator": { "open_id": operator },
        "action": { "value": card::encode_action_value(msg_id, choice_index) },
    })
}

fn count_replies(storage: &Storage, original_id: &str) -> usize {
    let conn = storage.conn();
    conn.query_row(
        "SELECT COUNT(*) FROM messages WHERE reply_to_id = ?1",
        [original_id],
        |r| r.get::<_, i64>(0),
    )
    .unwrap() as usize
}

/// 收集卡片所有 markdown/div 文本内容（布局演进时断言不绑死元素索引）。
fn card_texts(card: &Value) -> Vec<String> {
    card["body"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| {
            e.get("content")
                .and_then(|c| c.as_str())
                .or_else(|| e.pointer("/text/content").and_then(|c| c.as_str()))
                .map(|s| s.to_string())
        })
        .collect()
}

#[test]
fn card_action_wins_arbitration_and_returns_terminal_card() {
    let (storage, human_addr) = setup();
    let msg = send_approval(&storage, &human_addr);
    let data = card_event(&msg.id, 0, "ou_user");
    let out = decide_card_action(&storage, "ou_user", &data, Some("evt-1"));
    match out.decision {
        CardDecision::TerminalCard(card) => {
            let texts = card_texts(&card);
            assert!(
                texts.iter().any(|t| t.contains("已收到你的选择：批准")),
                "{:?}",
                texts
            );
        }
        CardDecision::Ack => panic!("expected terminal card"),
    }
    // 新回复必须暴露给调用方，用于唤醒接收方 SSE + notify
    let reply = out.created.expect("新回复应暴露给调用方");
    assert_eq!(reply.to_address, msg.from_address);
    assert_eq!(reply.reply_to_id.as_deref(), Some(msg.id.as_str()));
    // 抢答收尾：本端胜出，暴露原消息 id 供关闭 popup 弹窗
    assert_eq!(out.settled.as_deref(), Some(msg.id.as_str()));
    assert_eq!(count_replies(&storage, &msg.id), 1);
}

#[test]
fn duplicate_event_id_replays_terminal_card_without_second_reply() {
    let (storage, human_addr) = setup();
    let msg = send_approval(&storage, &human_addr);
    let data = card_event(&msg.id, 1, "ou_user");
    let first = decide_card_action(&storage, "ou_user", &data, Some("evt-1"));
    assert!(matches!(first.decision, CardDecision::TerminalCard(_)));
    assert!(first.created.is_some(), "首次执行应产生新回复");
    let second = decide_card_action(&storage, "ou_user", &data, Some("evt-1"));
    assert!(
        matches!(second.decision, CardDecision::TerminalCard(_)),
        "重推应回放终态卡片"
    );
    assert!(second.created.is_none(), "幂等回放不得再次唤醒接收方");
    assert!(second.settled.is_none(), "幂等回放不得重复抢答收尾");
    assert_eq!(
        count_replies(&storage, &msg.id),
        1,
        "重复事件不得产生第二条回复"
    );
}

#[test]
fn losing_surface_gets_resolved_card() {
    let (storage, human_addr) = setup();
    let msg = send_approval(&storage, &human_addr);
    // popup 先胜出
    approval::reply(
        &storage,
        HumanReplyRequest {
            message_id: &msg.id,
            body: "go",
            choices: &["批准"],
            surface: "popup",
            external_event_id: None,
        },
    )
    .unwrap();
    let data = card_event(&msg.id, 0, "ou_user");
    let out = decide_card_action(&storage, "ou_user", &data, Some("evt-2"));
    match out.decision {
        CardDecision::TerminalCard(card) => {
            let texts = card_texts(&card);
            assert!(
                texts.iter().any(|t| t.contains("已由 popup 处理")),
                "{:?}",
                texts
            );
        }
        CardDecision::Ack => panic!("expected terminal card"),
    }
    assert!(out.created.is_none(), "仲裁落败不产生新回复");
    assert!(out.settled.is_none(), "仲裁落败不做抢答收尾");
    // 落败方的回复消息已被回滚，只有 popup 的一条
    assert_eq!(count_replies(&storage, &msg.id), 1);
}

#[test]
fn unbound_operator_is_acked_without_reply() {
    let (storage, human_addr) = setup();
    let msg = send_approval(&storage, &human_addr);
    let data = card_event(&msg.id, 0, "ou_stranger");
    let out = decide_card_action(&storage, "ou_user", &data, Some("evt-3"));
    assert!(matches!(out.decision, CardDecision::Ack));
    assert!(out.created.is_none());
    assert_eq!(count_replies(&storage, &msg.id), 0);
}

#[test]
fn undecodable_value_is_acked() {
    let (storage, _human_addr) = setup();
    let data = json!({
        "operator": { "open_id": "ou_user" },
        "action": { "value": { "garbage": true } },
    });
    let out = decide_card_action(&storage, "ou_user", &data, Some("evt-4"));
    assert!(matches!(out.decision, CardDecision::Ack));
    assert!(out.created.is_none());
}

#[test]
fn unknown_message_is_acked() {
    let (storage, _human_addr) = setup();
    let data = card_event("no-such-uuid", 0, "ou_user");
    let out = decide_card_action(&storage, "ou_user", &data, Some("evt-5"));
    assert!(matches!(out.decision, CardDecision::Ack));
    assert!(out.created.is_none());
}

fn msg_event(open_id: &str, chat_type: &str, message_type: &str, content: &str) -> Value {
    json!({
        "sender": { "sender_id": { "open_id": open_id } },
        "message": {
            "chat_type": chat_type,
            "message_type": message_type,
            "content": content,
        },
    })
}

#[test]
fn decide_message_bound_p2p_text_yields_compose() {
    let data = msg_event("ou_user", "p2p", "text", "{\"text\":\"  你好 nora  \"}");
    match decide_message("ou_user", &data) {
        InboundMessage::Compose { open_id, text } => {
            assert_eq!(open_id, "ou_user");
            assert_eq!(text, "你好 nora", "正文应 trim");
        }
        InboundMessage::Ignore => panic!("expected compose"),
    }
}

#[test]
fn decide_message_ignores_unbound_group_nontext_empty() {
    let p2p = msg_event("ou_user", "p2p", "text", "{\"text\":\"hi\"}");
    // 非绑定用户
    assert!(matches!(
        decide_message("ou_other", &p2p),
        InboundMessage::Ignore
    ));
    // 群聊不转入 human mailbox
    let group = msg_event("ou_user", "group", "text", "{\"text\":\"hi\"}");
    assert!(matches!(
        decide_message("ou_user", &group),
        InboundMessage::Ignore
    ));
    // 非文本
    let image = msg_event("ou_user", "p2p", "image", "{\"image_key\":\"k\"}");
    assert!(matches!(
        decide_message("ou_user", &image),
        InboundMessage::Ignore
    ));
    // 空文本 / 纯空白 / 非法 content
    for content in ["{\"text\":\"   \"}", "{\"text\":\"\"}", "not-json"] {
        let empty = msg_event("ou_user", "p2p", "text", content);
        assert!(
            matches!(decide_message("ou_user", &empty), InboundMessage::Ignore),
            "content={}",
            content
        );
    }
    // 未绑定配置（open_id 为空）不产出 compose（绑定发现由 run() 打日志）
    assert!(matches!(decide_message("", &p2p), InboundMessage::Ignore));
}

// ===== compose / 卡片内回复 =====

use crate::identity::mailbox::mark_left;

/// agent → human 发一条普通文本消息（回复入口挂在它的文本卡片上）。
fn send_text_to_human(storage: &Storage, human_addr: &str) -> Message {
    let agent = create(storage, "nora", "编程专家", "").unwrap();
    send(
        storage,
        SendRequest {
            to: human_addr,
            to_name: "human",
            from: &agent,
            from_name: "nora",
            body: "周报写完了",
            content_type: "text",
            reply_to_id: None,
            subject: None,
            metadata: "{}",
            more_coming: false,
        },
    )
    .unwrap()
}

fn compose_event(body: &str, target: &str, operator: &str) -> Value {
    json!({
        "operator": { "open_id": operator },
        "action": {
            "value": { "action": "compose_submit" },
            "form_value": { "body": body, "target": target },
        },
    })
}

fn reply_event(action: &str, msg_id: &str, body: Option<&str>, operator: &str) -> Value {
    let form = match body {
        Some(b) => json!({ "body": b }),
        None => Value::Null,
    };
    json!({
        "operator": { "open_id": operator },
        "action": {
            "value": { "action": action, "agtalk_msg": msg_id },
            "form_value": form,
        },
    })
}

fn inbox_count(storage: &Storage, address: &str) -> usize {
    crate::routing::inbox::inbox(storage, address, true)
        .unwrap()
        .len()
}

#[test]
fn active_compose_agents_excludes_human_and_left() {
    let (storage, _human_addr) = setup();
    create(&storage, "nora", "编程专家", "").unwrap();
    let left = create(&storage, "gone", "", "").unwrap();
    mark_left(&storage, &left).unwrap();
    let agents = active_compose_agents(&storage);
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name, "nora");
    assert_eq!(agents[0].intro, "编程专家");
}

#[test]
fn compose_submit_delivers_to_agent_inbox() {
    let (storage, _human_addr) = setup();
    let agent = create(&storage, "nora", "编程专家", "").unwrap();
    let data = compose_event("看下这个 PR", &agent, "ou_user");
    let out = decide_card_action(&storage, "ou_user", &data, Some("evt-c1"));
    match &out.decision {
        CardDecision::TerminalCard(card) => {
            let texts = card_texts(card);
            assert!(
                texts.iter().any(|t| t.contains("已发送给 nora")),
                "{:?}",
                texts
            );
        }
        CardDecision::Ack => panic!("expected terminal card"),
    }
    // 新消息暴露给调用方唤醒 SSE/notify；落在 agent inbox，from=human
    let msg = out.created.expect("compose 应产生新消息");
    assert_eq!(msg.to_address, agent);
    assert_eq!(msg.body, "看下这个 PR");
    assert_eq!(inbox_count(&storage, &agent), 1);
}

#[test]
fn compose_submit_duplicate_event_replays_without_second_message() {
    let (storage, _human_addr) = setup();
    let agent = create(&storage, "nora", "", "").unwrap();
    let data = compose_event("hi", &agent, "ou_user");
    let first = decide_card_action(&storage, "ou_user", &data, Some("evt-c2"));
    assert!(first.created.is_some());
    let second = decide_card_action(&storage, "ou_user", &data, Some("evt-c2"));
    assert!(
        matches!(second.decision, CardDecision::TerminalCard(_)),
        "重推应回放终态卡片"
    );
    assert!(second.created.is_none(), "幂等回放不得再次唤醒");
    assert_eq!(
        inbox_count(&storage, &agent),
        1,
        "重复事件不得产生第二条消息"
    );
}

#[test]
fn compose_submit_rejects_tampered_unknown_left_target() {
    let (storage, _human_addr) = setup();
    let left = create(&storage, "gone", "", "").unwrap();
    mark_left(&storage, &left).unwrap();
    for target in ["no-such-uuid".to_string(), left.clone()] {
        let data = compose_event("hi", &target, "ou_user");
        let out = decide_card_action(&storage, "ou_user", &data, Some("evt-c3"));
        match &out.decision {
            CardDecision::TerminalCard(card) => {
                let texts = card_texts(card);
                assert!(
                    texts
                        .iter()
                        .any(|t| t.contains("目标 agent 不存在或已离开")),
                    "{:?}",
                    texts
                );
            }
            CardDecision::Ack => panic!("expected error terminal card"),
        }
        assert!(out.created.is_none());
        assert_eq!(inbox_count(&storage, &target), 0);
    }
}

#[test]
fn compose_submit_empty_body_returns_error_card() {
    let (storage, _human_addr) = setup();
    let agent = create(&storage, "nora", "", "").unwrap();
    let data = compose_event("   ", &agent, "ou_user");
    let out = decide_card_action(&storage, "ou_user", &data, Some("evt-c4"));
    match &out.decision {
        CardDecision::TerminalCard(card) => {
            let texts = card_texts(card);
            assert!(texts.iter().any(|t| t.contains("正文为空")), "{:?}", texts);
        }
        CardDecision::Ack => panic!("expected error terminal card"),
    }
    assert!(out.created.is_none());
    assert_eq!(inbox_count(&storage, &agent), 0);
}

#[test]
fn reply_open_returns_form_card() {
    let (storage, human_addr) = setup();
    let msg = send_text_to_human(&storage, &human_addr);
    let data = reply_event("reply_open", &msg.id, None, "ou_user");
    let out = decide_card_action(&storage, "ou_user", &data, Some("evt-r1"));
    match &out.decision {
        CardDecision::TerminalCard(card) => {
            let texts = card_texts(card);
            assert!(texts.iter().any(|t| t.contains("回复 nora")), "{:?}", texts);
            assert!(
                texts.iter().any(|t| t.contains("周报写完了")),
                "{:?}",
                texts
            );
        }
        CardDecision::Ack => panic!("expected form card"),
    }
    assert!(out.created.is_none(), "打开表单无副作用");
}

#[test]
fn reply_submit_creates_reply_with_reply_to_id() {
    let (storage, human_addr) = setup();
    let msg = send_text_to_human(&storage, &human_addr);
    let data = reply_event("reply_submit", &msg.id, Some("收到，辛苦"), "ou_user");
    let out = decide_card_action(&storage, "ou_user", &data, Some("evt-r2"));
    match &out.decision {
        CardDecision::TerminalCard(card) => {
            let texts = card_texts(card);
            assert!(
                texts.iter().any(|t| t.contains("已回复 nora")),
                "{:?}",
                texts
            );
            // 终态卡必须保留原消息上下文（回复的是哪条消息）
            assert!(
                texts.iter().any(|t| t.contains(&msg.body)),
                "终态卡应包含原消息正文: {:?}",
                texts
            );
            assert!(
                texts.iter().any(|t| t.contains("收到，辛苦")),
                "终态卡应包含回复正文: {:?}",
                texts
            );
        }
        CardDecision::Ack => panic!("expected terminal card"),
    }
    let reply = out.created.expect("回复应产生新消息");
    assert_eq!(reply.reply_to_id.as_deref(), Some(msg.id.as_str()));
    assert_eq!(reply.to_address, msg.from_address);
    assert_eq!(reply.body, "收到，辛苦");
    // 抢答收尾：本端处理了原消息，暴露给调用方关闭 popup 弹窗
    assert_eq!(out.settled.as_deref(), Some(msg.id.as_str()));
    // 重复 event_id：不重复创建、不重复收尾
    let dup = decide_card_action(&storage, "ou_user", &data, Some("evt-r2"));
    assert!(dup.created.is_none());
    assert!(dup.settled.is_none());
    assert_eq!(count_replies(&storage, &msg.id), 1);
}

#[test]
fn reply_submit_empty_body_returns_error_card() {
    let (storage, human_addr) = setup();
    let msg = send_text_to_human(&storage, &human_addr);
    let data = reply_event("reply_submit", &msg.id, Some("  "), "ou_user");
    let out = decide_card_action(&storage, "ou_user", &data, Some("evt-r3"));
    assert!(matches!(out.decision, CardDecision::TerminalCard(_)));
    assert!(out.created.is_none());
    assert_eq!(count_replies(&storage, &msg.id), 0);
}

#[test]
fn link_status_defaults_disconnected() {
    let link = LinkStatus::default();
    assert!(!link.is_connected());
}

// ===== 审批表单提交 / 完成 / 取消 =====

fn approval_submit_event(msg_id: &str, form_value: Value, operator: &str) -> Value {
    json!({
        "operator": { "open_id": operator },
        "action": {
            "value": { "action": "approval_submit", "agtalk_msg": msg_id },
            "form_value": form_value,
        },
    })
}

fn send_multi_approval(storage: &Storage, human_addr: &str, metadata: Value) -> Message {
    let agent = create(storage, "agent", "", "").unwrap();
    send(
        storage,
        SendRequest {
            to: human_addr,
            to_name: "human",
            from: &agent,
            from_name: "agent",
            body: "选哪些？",
            content_type: "approval_request",
            reply_to_id: None,
            subject: None,
            metadata: &metadata.to_string(),
            more_coming: false,
        },
    )
    .unwrap()
}

#[test]
fn approval_submit_multi_select_joins_choices_with_supplement() {
    let (storage, human_addr) = setup();
    let msg = send_multi_approval(&storage, &human_addr, json!({ "choices": ["a", "b", "c"] }));
    let data = approval_submit_event(
        &msg.id,
        json!({ "opt_0": true, "opt_2": true, "body": "尽快" }),
        "ou_user",
    );
    let out = decide_card_action(&storage, "ou_user", &data, Some("evt-a1"));
    match &out.decision {
        CardDecision::TerminalCard(card) => {
            let texts = card_texts(card);
            assert!(
                texts.iter().any(|t| t.contains("已收到你的选择：a、c")),
                "{:?}",
                texts
            );
            // 终态卡回显多选
            assert!(texts.iter().any(|t| t.contains("✅ a")), "{:?}", texts);
            assert!(texts.iter().any(|t| t.contains("⬜ b")), "{:?}", texts);
            assert!(texts.iter().any(|t| t.contains("✅ c")), "{:?}", texts);
        }
        CardDecision::Ack => panic!("expected terminal card"),
    }
    let reply = out.created.expect("审批提交应产生新回复");
    assert_eq!(reply.content_type, "approval_response");
    assert_eq!(reply.metadata, r#"{"choice":"a、c"}"#);
    assert_eq!(reply.body, "选择：a、c\n尽快");
    assert_eq!(out.settled.as_deref(), Some(msg.id.as_str()));
    // resolution 落库为多选 join
    let conn = storage.conn();
    let selected: String = conn
        .query_row(
            "SELECT selected_choice FROM approval_resolutions WHERE request_message_id = ?1",
            [&msg.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(selected, "a、c");
}

#[test]
fn approval_submit_select_only_requires_choice() {
    let (storage, human_addr) = setup();
    let msg = send_approval(&storage, &human_addr);
    let data = approval_submit_event(&msg.id, json!({}), "ou_user");
    let out = decide_card_action(&storage, "ou_user", &data, Some("evt-a2"));
    match &out.decision {
        CardDecision::TerminalCard(card) => {
            let texts = card_texts(card);
            assert!(
                texts.iter().any(|t| t.contains("请至少选择一个选项")),
                "{:?}",
                texts
            );
        }
        CardDecision::Ack => panic!("expected error terminal card"),
    }
    assert!(out.created.is_none());
    assert_eq!(count_replies(&storage, &msg.id), 0);
}

#[test]
fn approval_submit_single_rejects_multiple_choices() {
    let (storage, human_addr) = setup();
    let msg = send_multi_approval(
        &storage,
        &human_addr,
        json!({ "choices": ["a", "b"], "single": true }),
    );
    let data = approval_submit_event(&msg.id, json!({ "opt_0": true, "opt_1": true }), "ou_user");
    let out = decide_card_action(&storage, "ou_user", &data, Some("evt-a3"));
    match &out.decision {
        CardDecision::TerminalCard(card) => {
            let texts = card_texts(card);
            assert!(
                texts.iter().any(|t| t.contains("单选问题只能选择一个选项")),
                "{:?}",
                texts
            );
        }
        CardDecision::Ack => panic!("expected error terminal card"),
    }
    assert!(out.created.is_none());
    assert_eq!(count_replies(&storage, &msg.id), 0);
}

#[test]
fn approval_submit_on_text_message_is_acked() {
    let (storage, human_addr) = setup();
    let msg = send_text_to_human(&storage, &human_addr);
    let data = approval_submit_event(&msg.id, json!({ "opt_0": true }), "ou_user");
    let out = decide_card_action(&storage, "ou_user", &data, Some("evt-a4"));
    assert!(matches!(out.decision, CardDecision::Ack));
    assert!(out.created.is_none());
}

#[test]
fn done_marks_message_done_and_settles() {
    let (storage, human_addr) = setup();
    let msg = send_text_to_human(&storage, &human_addr);
    let data = reply_event("done", &msg.id, None, "ou_user");
    let out = decide_card_action(&storage, "ou_user", &data, Some("evt-d1"));
    match &out.decision {
        CardDecision::TerminalCard(card) => {
            let texts = card_texts(card);
            assert!(texts.iter().any(|t| t.contains("已完成")), "{:?}", texts);
        }
        CardDecision::Ack => panic!("expected terminal card"),
    }
    assert!(out.created.is_none(), "完成不产生回复消息");
    assert_eq!(out.settled.as_deref(), Some(msg.id.as_str()));
    let status: String = storage
        .conn()
        .query_row(
            "SELECT status FROM messages WHERE id = ?1",
            [&msg.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "done");
    // 重复事件：回放终态卡，不重复收尾
    let dup = decide_card_action(&storage, "ou_user", &data, Some("evt-d1"));
    assert!(matches!(dup.decision, CardDecision::TerminalCard(_)));
    assert!(dup.settled.is_none());
}

#[test]
fn cancel_creates_cancelled_reply_and_marks_done() {
    let (storage, human_addr) = setup();
    let msg = send_text_to_human(&storage, &human_addr);
    let data = reply_event("cancel", &msg.id, None, "ou_user");
    let out = decide_card_action(&storage, "ou_user", &data, Some("evt-x1"));
    match &out.decision {
        CardDecision::TerminalCard(card) => {
            let texts = card_texts(card);
            assert!(
                texts.iter().any(|t| t.contains("已取消（已通知 nora）")),
                "{:?}",
                texts
            );
        }
        CardDecision::Ack => panic!("expected terminal card"),
    }
    let reply = out.created.expect("取消应产生通知回复");
    assert_eq!(reply.body, "（已取消）");
    assert_eq!(reply.metadata, r#"{"cancelled":true}"#);
    assert_eq!(reply.to_address, msg.from_address);
    assert_eq!(out.settled.as_deref(), Some(msg.id.as_str()));
    let status: String = storage
        .conn()
        .query_row(
            "SELECT status FROM messages WHERE id = ?1",
            [&msg.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "done");
    // 重复事件：回放，不重复创建
    let dup = decide_card_action(&storage, "ou_user", &data, Some("evt-x1"));
    assert!(dup.created.is_none());
    assert!(dup.settled.is_none());
    assert_eq!(count_replies(&storage, &msg.id), 1);
}

#[test]
fn cancel_on_approval_blocks_later_submit() {
    let (storage, human_addr) = setup();
    let msg = send_approval(&storage, &human_addr);
    let cancel = reply_event("cancel", &msg.id, None, "ou_user");
    let out = decide_card_action(&storage, "ou_user", &cancel, Some("evt-x2"));
    assert!(out.created.is_some());
    // 取消后再提交审批表单：被仲裁拒绝
    let submit = approval_submit_event(&msg.id, json!({ "opt_0": true }), "ou_user");
    let out2 = decide_card_action(&storage, "ou_user", &submit, Some("evt-x3"));
    match &out2.decision {
        CardDecision::TerminalCard(card) => {
            let texts = card_texts(card);
            assert!(
                texts.iter().any(|t| t.contains("已由 feishu 处理")),
                "{:?}",
                texts
            );
        }
        CardDecision::Ack => panic!("expected resolved terminal card"),
    }
    assert!(out2.created.is_none());
    assert_eq!(count_replies(&storage, &msg.id), 1);
}
