//! 入站事件决策（IO 无关，可测）：卡片回调动作分派 + p2p 文本入站判定。
//!
//! 动作：
//! - approval：value 精确路由 → receipt 幂等 reply → 终态卡片（仲裁语义不变）。
//! - compose_submit：form_value 取正文/目标，服务端重验活跃 agent，复用 human 发信幂等。
//! - reply_open / reply_submit：卡片内回复表单，复用 human reply 路径。
//! - p2p 文本：仅绑定 open_id 的私聊文本产生 compose 草稿卡；群聊/非文本/空文本忽略。

use super::{card, SURFACE};
use crate::human::approval::{self, HumanReplyRequest};
use crate::human::HumanError;
use crate::routing::{Message, SendRequest};
use crate::storage::Storage;
use rusqlite::OptionalExtension;
use serde_json::Value;
use tracing::warn;

/// 卡片回调的处理决策（IO 无关，可测）。
pub enum CardDecision {
    /// 空 ACK：value 无法解析、消息不存在、非绑定用户等。
    Ack,
    /// 回包换卡：终态卡片 JSON。
    TerminalCard(Value),
}

/// 卡片回调处理结果：回包决策 + 新创建的消息（compose 发送 / 回复）。
pub struct CardActionOutcome {
    pub decision: CardDecision,
    /// 新创建的消息（幂等回放、仲裁落败、忽略场景为 None）；
    /// 调用方据此唤醒接收方 SSE + notify。
    pub created: Option<Message>,
}

fn ack() -> CardActionOutcome {
    CardActionOutcome {
        decision: CardDecision::Ack,
        created: None,
    }
}

fn terminal(card: Value) -> CardActionOutcome {
    CardActionOutcome {
        decision: CardDecision::TerminalCard(card),
        created: None,
    }
}

/// 卡片回调决策：operator 校验 → 动作分派。
/// 各动作内部做 receipt 幂等；幂等回放也返回终态卡片（飞书重推时视觉收敛）。
pub fn decide_card_action(
    storage: &Storage,
    bound_open_id: &str,
    data: &Value,
    event_id: Option<&str>,
) -> CardActionOutcome {
    // operator 校验：v1 单用户，仅绑定 open_id 的点击生效
    let operator = data
        .pointer("/operator/open_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !bound_open_id.is_empty() && operator != bound_open_id {
        warn!("feishu 卡片回调来自未绑定 open_id: {}", operator);
        return ack();
    }
    let value = data
        .pointer("/action/value")
        .cloned()
        .unwrap_or(Value::Null);
    match card::decode_action(&value) {
        Some(card::CardAction::Approval {
            msg_id,
            choice_index,
        }) => decide_approval(storage, event_id, &msg_id, choice_index),
        Some(card::CardAction::ComposeSubmit) => decide_compose_submit(storage, data, event_id),
        Some(card::CardAction::ReplyOpen { msg_id }) => decide_reply_open(storage, &msg_id),
        Some(card::CardAction::ReplySubmit { msg_id }) => {
            decide_reply_submit(storage, data, event_id, &msg_id)
        }
        None => {
            warn!("feishu 卡片回调 value 无法解析");
            ack()
        }
    }
}

/// 审批选择：value 精确路由 → receipt 幂等 reply → 终态卡片（仲裁语义不变）。
fn decide_approval(
    storage: &Storage,
    event_id: Option<&str>,
    msg_id: &str,
    idx: usize,
) -> CardActionOutcome {
    let msg = match query_message(storage, msg_id) {
        Ok(Some(m)) => m,
        _ => return ack(),
    };
    let choices = card::approval_choices(&msg);
    let Some(choice) = choices.get(idx).cloned() else {
        return ack();
    };
    let body = format!("选择：{}", choice);
    let req = HumanReplyRequest {
        message_id: msg_id,
        body: &body,
        choice: Some(&choice),
        surface: SURFACE,
        external_event_id: event_id,
    };
    match approval::reply(storage, req) {
        Ok(out) => CardActionOutcome {
            decision: CardDecision::TerminalCard(card::terminal_card(
                &msg,
                &format!("已收到你的选择：{}", choice),
                Some(&choice),
            )),
            // 幂等回放未创建新消息，不重复唤醒接收方
            created: if out.deduplicated {
                None
            } else {
                Some(out.reply)
            },
        },
        Err(HumanError::AlreadyResolved { resolved_by, .. }) => terminal(card::terminal_card(
            &msg,
            &format!("已由 {} 处理", resolved_by),
            None,
        )),
        Err(e) => {
            warn!("feishu 卡片回调 reply 失败: {}", e);
            ack()
        }
    }
}

/// compose 提交：form_value 取正文/目标 → 服务端重验目标是活跃 agent →
/// 复用 human→agent 发信 + receipt 幂等。不信任卡片 payload。
fn decide_compose_submit(
    storage: &Storage,
    data: &Value,
    event_id: Option<&str>,
) -> CardActionOutcome {
    let form = data
        .pointer("/action/form_value")
        .cloned()
        .unwrap_or(Value::Null);
    let body = form
        .get(card::FORM_BODY)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let target = form
        .get(card::FORM_TARGET)
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if body.is_empty() {
        return terminal(card::status_card(
            "选择 Agent 并发送",
            "",
            "消息正文为空，未发送",
        ));
    }
    let human = match crate::human::human_address(storage) {
        Ok(a) => a,
        Err(e) => {
            warn!("feishu compose 取 human 地址失败: {}", e);
            return ack();
        }
    };
    // 服务端再次验证：活跃 mailbox、非 human 自身（客户端可篡改 option value）
    let mb = match crate::identity::mailbox::get_by_address(storage, target) {
        Ok(Some(mb)) if mb.left_at.is_none() && mb.address != human => mb,
        _ => {
            return terminal(card::status_card(
                "选择 Agent 并发送",
                &body,
                "发送失败：目标 agent 不存在或已离开",
            ))
        }
    };
    let req = SendRequest {
        to: &mb.address,
        to_name: &mb.name,
        from: &human,
        from_name: "human",
        body: &body,
        content_type: "text",
        reply_to_id: None,
        subject: None,
        metadata: "{}",
        more_coming: false,
    };
    let result: Result<(Message, bool), String> = match event_id {
        Some(eid) => {
            crate::human::send_with_receipt(storage, &req, SURFACE, eid).map_err(|e| e.to_string())
        }
        None => crate::routing::send::send(storage, req)
            .map(|m| (m, false))
            .map_err(|e| e.to_string()),
    };
    match result {
        Ok((msg, deduplicated)) => CardActionOutcome {
            decision: CardDecision::TerminalCard(card::status_card(
                "选择 Agent 并发送",
                &body,
                &format!("已发送给 {}", mb.name),
            )),
            created: if deduplicated { None } else { Some(msg) },
        },
        Err(e) => {
            warn!("feishu compose 发送失败: {}", e);
            ack()
        }
    }
}

/// 打开回复表单：卡片原地切换为 reply_form_card（无副作用，不需幂等）。
fn decide_reply_open(storage: &Storage, msg_id: &str) -> CardActionOutcome {
    match query_message(storage, msg_id) {
        Ok(Some(m)) => terminal(card::reply_form_card(&m)),
        _ => ack(),
    }
}

/// 回复提交：form_value 取正文 → 复用 human reply 路径（reply_to_id/仲裁/幂等）。
fn decide_reply_submit(
    storage: &Storage,
    data: &Value,
    event_id: Option<&str>,
    msg_id: &str,
) -> CardActionOutcome {
    let original = match query_message(storage, msg_id) {
        Ok(Some(m)) => m,
        _ => return ack(),
    };
    let title = format!("回复 {}", original.from_name);
    let body = data
        .pointer("/action/form_value")
        .and_then(|v| v.get(card::FORM_BODY))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if body.is_empty() {
        return terminal(card::status_card(&title, "", "回复内容为空，未发送"));
    }
    let req = HumanReplyRequest {
        message_id: msg_id,
        body: &body,
        choice: None,
        surface: SURFACE,
        external_event_id: event_id,
    };
    match approval::reply(storage, req) {
        Ok(out) => CardActionOutcome {
            decision: CardDecision::TerminalCard(card::status_card(
                &title,
                &body,
                &format!("已回复 {}", original.from_name),
            )),
            created: if out.deduplicated {
                None
            } else {
                Some(out.reply)
            },
        },
        Err(e) => {
            warn!("feishu 卡片内回复失败: {}", e);
            terminal(card::status_card(
                &title,
                &body,
                &format!("发送失败：{}", e),
            ))
        }
    }
}

/// p2p 文本入站决策。
pub enum InboundMessage {
    /// 忽略：非绑定用户 / 非 p2p / 非文本 / 空文本。
    Ignore,
    /// 绑定用户的 p2p 文本 → 回复 compose 草稿卡（预填正文）。
    Compose { open_id: String, text: String },
}

/// 消息事件决策：仅绑定 open_id 的 p2p 文本产生 compose；其余安全忽略。
/// 不做群聊路由，不做自由文字归属猜测。
pub fn decide_message(bound_open_id: &str, data: &Value) -> InboundMessage {
    let Some(open_id) = data
        .pointer("/sender/sender_id/open_id")
        .and_then(|v| v.as_str())
    else {
        return InboundMessage::Ignore;
    };
    if bound_open_id.is_empty() || open_id != bound_open_id {
        return InboundMessage::Ignore;
    }
    if data.pointer("/message/chat_type").and_then(|v| v.as_str()) != Some("p2p") {
        return InboundMessage::Ignore;
    }
    if data
        .pointer("/message/message_type")
        .and_then(|v| v.as_str())
        != Some("text")
    {
        return InboundMessage::Ignore;
    }
    let content = data
        .pointer("/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let text = serde_json::from_str::<Value>(content)
        .ok()
        .and_then(|c| {
            c.get("text")
                .and_then(|t| t.as_str())
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_default();
    if text.is_empty() {
        return InboundMessage::Ignore;
    }
    InboundMessage::Compose {
        open_id: open_id.to_string(),
        text,
    }
}

/// compose 下拉候选：活跃 mailbox，排除 human 自身。
pub(super) fn active_compose_agents(storage: &Storage) -> Vec<card::ComposeAgent> {
    let human = crate::human::human_address(storage).unwrap_or_default();
    crate::routing::lookup::lookup(storage, None)
        .map(|mbs| {
            mbs.into_iter()
                .filter(|mb| mb.left_at.is_none() && mb.address != human)
                .map(|mb| card::ComposeAgent {
                    address: mb.address,
                    name: mb.name,
                    intro: mb.intro,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn query_message(storage: &Storage, id: &str) -> Result<Option<Message>, HumanError> {
    let conn = storage.conn();
    let msg = conn
        .query_row(
            "SELECT id, to_address, to_name, from_address, from_name, body, content_type, \
             reply_to_id, subject, metadata, event_id, status, created_at \
             FROM messages WHERE id = ?1",
            [id],
            Message::from_row,
        )
        .optional()?;
    Ok(msg)
}

#[cfg(test)]
mod tests;
