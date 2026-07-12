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
/// 兼容飞书把 value 作为 JSON 字符串回传的情况。
pub fn decode_action_value(value: &Value) -> Option<(String, usize)> {
    let obj = match value {
        Value::String(s) => serde_json::from_str::<Value>(s).ok()?,
        v => v.clone(),
    };
    let msg = obj.get(ACTION_MSG_KEY)?.as_str()?.to_string();
    let idx = obj.get(ACTION_CHOICE_KEY)?.as_u64()? as usize;
    Some((msg, idx))
}

fn skeleton(elements: Vec<Value>) -> Value {
    json!({
        "schema": "2.0",
        "config": { "update_multi": true },
        "body": { "elements": elements },
    })
}

/// 样式化头部行：蓝色小字 + maybe_filled 图标 + hr（对齐 AskHuman 生产样式，
/// 不用原生大 banner——标题字体固定且渲染成整条色带）。
fn styled_header(title: &str) -> Vec<Value> {
    vec![
        json!({
            "tag": "div",
            "text": {
                "tag": "plain_text",
                "content": title,
                "text_size": "notation",
                "text_align": "left",
                "text_color": "blue",
            },
            "icon": { "tag": "standard_icon", "token": "maybe_filled", "color": "blue" },
            "margin": "0px 0px 0px 0px",
        }),
        json!({ "tag": "hr", "margin": "0px 0px 0px 0px" }),
    ]
}

/// 审批卡片：样式化头部（来源 agent）+ 正文 + choices 按钮行。
/// Card JSON 2.0：按钮直接作为元素（column_set 横向排列），
/// 回调数据放 behaviors callback value——V2 已不支持 V1 的 action 容器与按钮顶层 value。
/// recommended 选项的按钮 primary 高亮；无 recommended 时首个按钮 primary。
pub fn approval_card(msg: &Message, choices: &[String]) -> Value {
    let recommended = approval_recommended(msg);
    let mut elements = styled_header(&format!("来自 {} 的审批", msg.from_name));
    elements.push(json!({ "tag": "markdown", "content": msg.body }));
    elements.push(json!({ "tag": "hr" }));
    let columns: Vec<Value> = choices
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let primary = match &recommended {
                Some(r) => r == c,
                None => i == 0,
            };
            json!({
                "tag": "column",
                "width": "auto",
                "elements": [json!({
                    "tag": "button",
                    "text": { "tag": "plain_text", "content": c },
                    "type": if primary { "primary" } else { "default" },
                    "behaviors": [
                        { "type": "callback", "value": encode_action_value(&msg.id, i) }
                    ],
                })],
            })
        })
        .collect();
    elements.push(json!({
        "tag": "column_set",
        "horizontal_spacing": "8px",
        "columns": columns,
    }));
    skeleton(elements)
}

/// 文本卡片：样式化头部（来源 agent）+ markdown 正文。
pub fn text_card(from_name: &str, body: &str) -> Value {
    let mut elements = styled_header(&format!("来自 {} 的消息", from_name));
    elements.push(json!({ "tag": "markdown", "content": body }));
    skeleton(elements)
}

/// 终态卡片：样式化头部 + 正文 + 选项回显（选中项 ✅）+ 状态行。
pub fn terminal_card(msg: &Message, status_line: &str, selected: Option<&str>) -> Value {
    let mut elements = styled_header(&format!("来自 {} 的审批", msg.from_name));
    elements.push(json!({ "tag": "markdown", "content": msg.body }));
    elements.push(json!({ "tag": "hr" }));
    let choices = approval_choices(msg);
    if !choices.is_empty() {
        let lines: Vec<String> = choices
            .iter()
            .map(|c| {
                if Some(c.as_str()) == selected {
                    format!("✅ {}", c)
                } else {
                    format!("⬜ {}", c)
                }
            })
            .collect();
        elements.push(json!({ "tag": "markdown", "content": lines.join("\n") }));
    }
    elements.push(json!({
        "tag": "markdown",
        "content": format!("**{}**", status_line),
    }));
    skeleton(elements)
}

/// 从审批消息 metadata 取 recommended 选项原文。
pub fn approval_recommended(msg: &Message) -> Option<String> {
    serde_json::from_str::<Value>(&msg.metadata)
        .ok()
        .and_then(|m| {
            m.get("recommended")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
}

/// 卡片回调的同步「更新卡片」回包体：`{card:{type:"raw",data:<新卡片>}}`。
/// 长连接回包必须带这层包装，飞书才会把按钮卡片原地换成终态卡片；
/// 裸卡片 JSON 会被忽略——按钮无反应且可重复点击。
pub fn callback_update_card(card: Value) -> Value {
    json!({ "card": { "type": "raw", "data": card } })
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
}
