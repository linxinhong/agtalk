//! card 的单元测试（就近原则；从 card.rs 拆出）。

use super::*;

fn approval_msg() -> Message {
    Message {
        id: "8c40699c-1111-2222-3333-444455556666".to_string(),
        to_address: "human".to_string(),
        to_name: "human".to_string(),
        from_address: "agent".to_string(),
        from_name: "nora".to_string(),
        body: "部署到生产？".to_string(),
        content_type: "approval_request".to_string(),
        reply_to_id: None,
        subject: None,
        metadata: json!({ "choices": ["批准", "拒绝"], "select_only": true }).to_string(),
        event_id: 1,
        status: "pending".to_string(),
        created_at: 0.0,
    }
}

#[test]
fn action_value_roundtrip() {
    let v = encode_action_value("uuid-1", 3);
    assert_eq!(decode_action_value(&v), Some(("uuid-1".to_string(), 3)));
}

#[test]
fn decode_rejects_bad_shape() {
    assert_eq!(decode_action_value(&json!({})), None);
    assert_eq!(decode_action_value(&json!({ "agtalk_msg": "x" })), None);
    assert_eq!(
        decode_action_value(&json!({ "agtalk_msg": 1, "choice_index": 0 })),
        None
    );
}

#[test]
fn approval_card_has_button_per_choice_with_route_value() {
    let msg = approval_msg();
    let card = approval_card(&msg, &approval_choices(&msg));
    assert_eq!(card["schema"], "2.0");
    // 布局：头部行 + hr + 正文 + hr + column_set
    let header = &card["body"]["elements"][0];
    assert_eq!(header["tag"], "div");
    assert_eq!(header["text"]["content"], "来自 nora 的审批");
    let columns = card["body"]["elements"][4]["columns"].as_array().unwrap();
    assert_eq!(columns.len(), 2);
    let btn0 = &columns[0]["elements"][0];
    assert_eq!(btn0["tag"], "button");
    assert_eq!(btn0["type"], "primary");
    assert_eq!(
        btn0["behaviors"][0]["value"]["agtalk_msg"],
        serde_json::json!(msg.id)
    );
    assert_eq!(btn0["behaviors"][0]["value"]["choice_index"], 0);
    assert_eq!(
        columns[1]["elements"][0]["behaviors"][0]["value"]["choice_index"],
        1
    );
}

#[test]
fn recommended_choice_button_is_primary() {
    let mut msg = approval_msg();
    msg.metadata =
        json!({ "choices": ["批准", "拒绝"], "select_only": true, "recommended": "拒绝" })
            .to_string();
    let card = approval_card(&msg, &approval_choices(&msg));
    let columns = card["body"]["elements"][4]["columns"].as_array().unwrap();
    assert_eq!(columns[0]["elements"][0]["type"], "default");
    assert_eq!(columns[1]["elements"][0]["type"], "primary");
}

#[test]
fn decode_accepts_string_encoded_value() {
    let v = serde_json::json!("{\"agtalk_msg\":\"uuid-1\",\"choice_index\":2}");
    assert_eq!(decode_action_value(&v), Some(("uuid-1".to_string(), 2)));
}

#[test]
fn callback_update_card_wraps_raw_card() {
    let msg = approval_msg();
    let card = terminal_card(&msg, "已收到你的选择：批准", Some("批准"));
    let body = callback_update_card(card.clone());
    assert_eq!(body["card"]["type"], "raw");
    assert_eq!(body["card"]["data"], card);
}

#[test]
fn approval_choices_reads_metadata() {
    let msg = approval_msg();
    assert_eq!(approval_choices(&msg), vec!["批准", "拒绝"]);
}

#[test]
fn decode_action_distinguishes_all_actions() {
    // 审批（无 action 字段，向后兼容）
    let v = encode_action_value("uuid-1", 2);
    match decode_action(&v) {
        Some(CardAction::Approval {
            msg_id,
            choice_index,
        }) => {
            assert_eq!(msg_id, "uuid-1");
            assert_eq!(choice_index, 2);
        }
        _ => panic!("expected approval"),
    }
    assert!(matches!(
        decode_action(&json!({ "action": "compose_submit" })),
        Some(CardAction::ComposeSubmit)
    ));
    assert!(matches!(
        decode_action(&json!({ "action": "reply_open", "agtalk_msg": "m-1" })),
        Some(CardAction::ReplyOpen { msg_id }) if msg_id == "m-1"
    ));
    assert!(matches!(
        decode_action(&json!({ "action": "reply_submit", "agtalk_msg": "m-2" })),
        Some(CardAction::ReplySubmit { msg_id }) if msg_id == "m-2"
    ));
    // 未知动作 / 缺 msg id / 字符串编码
    assert!(matches!(decode_action(&json!({ "action": "nope" })), None));
    assert!(matches!(
        decode_action(&json!({ "action": "reply_open" })),
        None
    ));
    let s = json!("{\"action\":\"reply_open\",\"agtalk_msg\":\"m-3\"}");
    assert!(matches!(
        decode_action(&s),
        Some(CardAction::ReplyOpen { msg_id }) if msg_id == "m-3"
    ));
}

#[test]
fn compose_card_options_use_uuid_value_with_name_intro_label() {
    let agents = vec![
        ComposeAgent {
            address: "a1b2c3d4-1111-2222-3333-444455556666".into(),
            name: "nora".into(),
            intro: "编程专家".into(),
        },
        ComposeAgent {
            address: "e5f6a7b8-1111-2222-3333-444455556666".into(),
            name: "bob".into(),
            intro: "".into(),
        },
    ];
    let card = compose_card("草稿正文", &agents);
    let form = &card["body"]["elements"][2];
    assert_eq!(form["tag"], "form");
    let options = form["elements"][1]["options"].as_array().unwrap();
    assert_eq!(options.len(), 2);
    // option value 是完整 UUID；可见文案是 name — intro，不含 UUID
    assert_eq!(options[0]["value"], agents[0].address);
    let label0 = options[0]["text"]["content"].as_str().unwrap();
    assert_eq!(label0, "nora — 编程专家");
    assert!(!label0.contains("a1b2c3d4"));
    // 无 intro 时只显示 name
    assert_eq!(options[1]["text"]["content"], "bob");
    // 正文预填
    assert_eq!(form["elements"][0]["default_value"], "草稿正文");
    // 提交按钮动作
    assert_eq!(
        form["elements"][2]["behaviors"][0]["value"]["action"],
        "compose_submit"
    );
}

#[test]
fn text_card_has_reply_entry_with_msg_id() {
    let msg = approval_msg();
    let card = text_card(&msg);
    let elements = card["body"]["elements"].as_array().unwrap();
    let btn = elements.last().unwrap();
    assert_eq!(btn["tag"], "button");
    assert_eq!(btn["behaviors"][0]["value"]["action"], "reply_open");
    assert_eq!(btn["behaviors"][0]["value"]["agtalk_msg"], msg.id);
}

#[test]
fn reply_form_card_locks_target_via_msg_id() {
    let msg = approval_msg();
    let card = reply_form_card(&msg);
    let texts: Vec<&str> = card["body"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| {
            e.get("content")
                .and_then(|c| c.as_str())
                .or_else(|| e.pointer("/text/content").and_then(|c| c.as_str()))
        })
        .collect();
    assert!(texts.iter().any(|t| t.contains("回复 nora")), "{:?}", texts);
    let form = card["body"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["tag"] == "form")
        .unwrap();
    let submit = form["elements"].as_array().unwrap().last().unwrap();
    assert_eq!(submit["behaviors"][0]["value"]["action"], "reply_submit");
    assert_eq!(submit["behaviors"][0]["value"]["agtalk_msg"], msg.id);
}

#[test]
fn terminal_card_contains_status_line_and_choice_echo() {
    let msg = approval_msg();
    let card = terminal_card(&msg, "已由 popup 处理", Some("批准"));
    let contents: Vec<&str> = card["body"]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e.get("content").and_then(|c| c.as_str()))
        .collect();
    assert!(
        contents.iter().any(|c| c.contains("已由 popup 处理")),
        "{:?}",
        contents
    );
    let echo = contents.iter().find(|c| c.contains("批准")).unwrap();
    assert!(echo.contains("✅ 批准"), "{}", echo);
    assert!(echo.contains("⬜ 拒绝"), "{}", echo);
}
