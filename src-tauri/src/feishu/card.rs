//! 飞书消息卡片 JSON 2.0：审批卡片（choices→按钮）/ 文本卡片 / 终态卡片。
//! 红线：按钮 value 只存 agtalk message UUID + choice index，不存业务结论。

use crate::routing::Message;
use serde_json::{json, Value};

pub const ACTION_MSG_KEY: &str = "agtalk_msg";
pub const ACTION_CHOICE_KEY: &str = "choice_index";

/// 编码按钮 value：精确路由键（UUID + choice index）。
pub fn encode_action_value(message_id: &str, choice_index: usize) -> Value {
    json!({ ACTION_MSG_KEY: message_id, ACTION_CHOICE_KEY: choice_index })
}

/// 解码按钮 value；形状不符返回 None（调用方按忽略处理）。
pub fn decode_action_value(value: &Value) -> Option<(String, usize)> {
    let msg = value.get(ACTION_MSG_KEY)?.as_str()?.to_string();
    let idx = value.get(ACTION_CHOICE_KEY)?.as_u64()? as usize;
    Some((msg, idx))
}

fn skeleton(elements: Vec<Value>) -> Value {
    json!({
        "schema": "2.0",
        "config": { "update_multi": true },
        "body": { "elements": elements },
    })
}

/// 审批卡片：正文 + choices 按钮行（首个按钮 primary）。
pub fn approval_card(msg: &Message, choices: &[String]) -> Value {
    let mut elements = vec![
        json!({ "tag": "markdown", "content": msg.body }),
        json!({ "tag": "hr" }),
    ];
    let buttons: Vec<Value> = choices
        .iter()
        .enumerate()
        .map(|(i, c)| {
            json!({
                "tag": "button",
                "text": { "tag": "plain_text", "content": c },
                "type": if i == 0 { "primary" } else { "default" },
                "value": encode_action_value(&msg.id, i),
            })
        })
        .collect();
    elements.push(json!({ "tag": "action", "actions": buttons }));
    skeleton(elements)
}

/// 文本卡片：普通 human 消息展示。
pub fn text_card(from_name: &str, body: &str) -> Value {
    skeleton(vec![json!({
        "tag": "markdown",
        "content": format!("**{}**：\n{}", from_name, body),
    })])
}

/// 终态卡片：仲裁收尾回写（「已收到你的选择」/「已由 X 处理」）。
pub fn terminal_card(original_body: &str, status_line: &str) -> Value {
    skeleton(vec![
        json!({ "tag": "markdown", "content": original_body }),
        json!({ "tag": "hr" }),
        json!({ "tag": "markdown", "content": format!("**{}**", status_line) }),
    ])
}

/// 从审批消息 metadata 取 choices 列表。
pub fn approval_choices(msg: &Message) -> Vec<String> {
    serde_json::from_str::<Value>(&msg.metadata)
        .ok()
        .and_then(|m| m.get("choices").and_then(|c| c.as_array()).cloned())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
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
        let buttons = card["body"]["elements"][2]["actions"].as_array().unwrap();
        assert_eq!(buttons.len(), 2);
        assert_eq!(buttons[0]["value"]["agtalk_msg"], msg.id);
        assert_eq!(buttons[0]["value"]["choice_index"], 0);
        assert_eq!(buttons[1]["value"]["choice_index"], 1);
        assert_eq!(buttons[0]["type"], "primary");
    }

    #[test]
    fn approval_choices_reads_metadata() {
        let msg = approval_msg();
        assert_eq!(approval_choices(&msg), vec!["批准", "拒绝"]);
    }

    #[test]
    fn terminal_card_contains_status_line() {
        let card = terminal_card("部署到生产？", "已由 popup 处理");
        let elements = card["body"]["elements"].as_array().unwrap();
        assert_eq!(elements.len(), 3);
        assert!(elements[2]["content"]
            .as_str()
            .unwrap()
            .contains("已由 popup 处理"));
    }
}
