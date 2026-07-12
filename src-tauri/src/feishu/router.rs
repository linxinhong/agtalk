//! daemon 全局 FeishuRouter：单条长连接，入站事件 → receipt 幂等 → approval 仲裁。
//!
//! - 卡片回调：value 精确路由（UUID + choice index，不做归属猜测），飞书 event_id
//!   经 `human_action_receipts` 幂等；3 秒窗口内回包换终态卡片。
//! - 自由文字：v1 不做归属猜测，回复提示文本，不落库不路由。
//! - v1 单用户：仅 `config.feishu.open_id` 绑定用户的点击/消息被处理。

use super::ws::{FeishuWs, WsEvent};
use super::{card, client::FeishuClient};
use crate::config::FeishuConfig;
use crate::human::approval::{self, HumanReplyRequest};
use crate::human::HumanError;
use crate::notify::NotifyLimiter;
use crate::routing::Message;
use crate::server::handlers::msg as msg_handlers;
use crate::storage::Storage;
use crate::transport::wake::{SseEvent, SubscriberRegistry};
use rusqlite::OptionalExtension;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

pub const SURFACE: &str = "feishu";
const FREE_TEXT_HINT: &str = "[agtalk] 请通过卡片按钮回复审批；自由文字暂不路由（v1）。";
const RECONNECT_DELAY: Duration = Duration::from_secs(5);

/// 长连接状态句柄（doctor 可读；daemon 与 Router 共享）。
#[derive(Clone, Default)]
pub struct LinkStatus(Arc<AtomicBool>);

impl LinkStatus {
    pub fn is_connected(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    fn set(&self, connected: bool) {
        self.0.store(connected, Ordering::Relaxed);
    }
}

pub struct FeishuRouter {
    storage: Storage,
    cfg: FeishuConfig,
    client: FeishuClient,
    link: LinkStatus,
    registry: SubscriberRegistry,
    notify_limiter: Arc<NotifyLimiter>,
    dot_agtalk: PathBuf,
}

impl FeishuRouter {
    pub fn new(
        storage: Storage,
        cfg: FeishuConfig,
        link: LinkStatus,
        registry: SubscriberRegistry,
        notify_limiter: Arc<NotifyLimiter>,
        dot_agtalk: PathBuf,
    ) -> Self {
        let client = FeishuClient::new(&cfg.base_url, &cfg.app_id, &cfg.app_secret);
        Self {
            storage,
            cfg,
            client,
            link,
            registry,
            notify_limiter,
            dot_agtalk,
        }
    }

    /// 主循环：连接 → 收事件 → 断线重连（无限重试，退避固定 5s；
    /// ws.recv 内部还有连接级重试）。
    pub async fn run(self: Arc<Self>) {
        let http = reqwest::Client::new();
        loop {
            match FeishuWs::connect(
                http.clone(),
                &self.cfg.base_url,
                &self.cfg.app_id,
                &self.cfg.app_secret,
            )
            .await
            {
                Err(e) => {
                    warn!("feishu 长连接建立失败: {}", e);
                    self.link.set(false);
                    tokio::time::sleep(RECONNECT_DELAY).await;
                    continue;
                }
                Ok(mut ws) => {
                    info!("feishu 长连接已建立");
                    self.link.set(true);
                    while let Some(ev) = ws.recv().await {
                        match ev {
                            WsEvent::CardAction {
                                data,
                                event_id,
                                frame,
                            } => {
                                let out = decide_card_action(
                                    &self.storage,
                                    &self.cfg.open_id,
                                    &data,
                                    event_id.as_deref(),
                                );
                                // 新回复落库后唤醒接收方（与 popup/GUI 回复同一套机制）
                                if let Some(reply) = &out.reply {
                                    self.wake_recipient(reply);
                                }
                                match out.decision {
                                    CardDecision::Ack => ws.respond_ack(&frame).await,
                                    CardDecision::TerminalCard(card) => {
                                        // 回包必须包 card:{type:"raw"} 才会原地更新卡片
                                        ws.respond_card(&frame, &card::callback_update_card(card))
                                            .await
                                    }
                                }
                            }
                            WsEvent::Message { data, .. } => {
                                if let Some(open_id) = decide_message(&self.cfg.open_id, &data) {
                                    // 绑定发现：未配置 open_id 时日志输出发送者，便于首次配置
                                    if self.cfg.open_id.is_empty() {
                                        info!(
                                            "feishu 收到消息来自 open_id: {}（未绑定，可用 agtalk config set feishu.open_id {} 绑定）",
                                            open_id, open_id
                                        );
                                    }
                                    if let Err(e) =
                                        self.client.send_text(&open_id, FREE_TEXT_HINT).await
                                    {
                                        warn!("feishu 自由文字提示发送失败: {}", e);
                                    }
                                }
                            }
                        }
                    }
                    self.link.set(false);
                    warn!("feishu 长连接断开，准备重连");
                }
            }
        }
    }

    /// 回复落库后唤醒接收方：SSE 推送 + notify 打扰 + 本地 history 记录。
    /// 与 popup/GUI 回复路径（server::handlers::human::after_human_reply）保持同一套机制，
    /// 否则飞书点击产生的 approval_response 只落库不推送，agent 侧 msg wait 会超时。
    fn wake_recipient(&self, msg: &Message) {
        self.registry.notify(
            &msg.to_address,
            SseEvent {
                message: msg.clone(),
            },
        );
        let storage = self.storage.clone();
        let to = msg.to_address.clone();
        let message_id = msg.id.clone();
        let limiter = self.notify_limiter.clone();
        tokio::spawn(async move {
            if let Err(e) =
                crate::notify::trigger(&storage, &to, "human", &message_id, &limiter, None).await
            {
                tracing::debug!("notify trigger skipped: {}", e);
            }
        });
        if let Some(root) =
            msg_handlers::participant_root(&self.storage, &self.dot_agtalk, &msg.to_address)
        {
            if let Err(e) = crate::identity::history::append_message(
                &root,
                &msg.to_name,
                &msg.to_address,
                "in",
                msg,
                "human.reply",
            ) {
                warn!("receiver history append failed: {}", e);
            }
        }
    }
}

/// 卡片回调的处理决策（IO 无关，可测）。
pub enum CardDecision {
    /// 空 ACK：value 无法解析、消息不存在、非绑定用户等。
    Ack,
    /// 回包换卡：终态卡片 JSON。
    TerminalCard(Value),
}

/// 卡片回调处理结果：回包决策 + 新创建的回复消息。
pub struct CardActionOutcome {
    pub decision: CardDecision,
    /// 新创建的回复消息（幂等回放、仲裁落败、忽略场景为 None）；
    /// 调用方据此唤醒接收方 SSE + notify。
    pub reply: Option<Message>,
}

fn ack() -> CardActionOutcome {
    CardActionOutcome {
        decision: CardDecision::Ack,
        reply: None,
    }
}

/// 卡片回调决策：解析 value → receipt 幂等 reply → 终态卡片。
/// 幂等回放也返回终态卡片（飞书重推时视觉收敛）。
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
    let Some((msg_id, idx)) = card::decode_action_value(&value) else {
        warn!("feishu 卡片回调 value 无法解析");
        return ack();
    };
    let msg = match query_message(storage, &msg_id) {
        Ok(Some(m)) => m,
        _ => return ack(),
    };
    let choices = card::approval_choices(&msg);
    let Some(choice) = choices.get(idx).cloned() else {
        return ack();
    };
    let body = format!("选择：{}", choice);
    let req = HumanReplyRequest {
        message_id: &msg_id,
        body: &body,
        choice: Some(&choice),
        surface: SURFACE,
        external_event_id: event_id,
    };
    match approval::reply(storage, req) {
        Ok(out) => CardActionOutcome {
            decision: CardDecision::TerminalCard(card::terminal_card(
                &msg.body,
                &format!("已收到你的选择：{}", choice),
            )),
            // 幂等回放未创建新消息，不重复唤醒接收方
            reply: if out.deduplicated {
                None
            } else {
                Some(out.reply)
            },
        },
        Err(HumanError::AlreadyResolved { resolved_by, .. }) => CardActionOutcome {
            decision: CardDecision::TerminalCard(card::terminal_card(
                &msg.body,
                &format!("已由 {} 处理", resolved_by),
            )),
            reply: None,
        },
        Err(e) => {
            warn!("feishu 卡片回调 reply 失败: {}", e);
            ack()
        }
    }
}

/// 自由文字事件：返回需要发送提示的 open_id（非绑定用户返回 None）。
pub fn decide_message(bound_open_id: &str, data: &Value) -> Option<String> {
    let open_id = data
        .pointer("/sender/sender_id/open_id")
        .and_then(|v| v.as_str())?;
    if !bound_open_id.is_empty() && open_id != bound_open_id {
        return None;
    }
    Some(open_id.to_string())
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
mod tests {
    use super::*;
    use crate::config::HumanConfig;
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

    #[test]
    fn card_action_wins_arbitration_and_returns_terminal_card() {
        let (storage, human_addr) = setup();
        let msg = send_approval(&storage, &human_addr);
        let data = card_event(&msg.id, 0, "ou_user");
        let out = decide_card_action(&storage, "ou_user", &data, Some("evt-1"));
        match out.decision {
            CardDecision::TerminalCard(card) => {
                let text = card["body"]["elements"][2]["content"].as_str().unwrap();
                assert!(text.contains("已收到你的选择：批准"), "{}", text);
            }
            CardDecision::Ack => panic!("expected terminal card"),
        }
        // 新回复必须暴露给调用方，用于唤醒接收方 SSE + notify
        let reply = out.reply.expect("新回复应暴露给调用方");
        assert_eq!(reply.to_address, msg.from_address);
        assert_eq!(reply.reply_to_id.as_deref(), Some(msg.id.as_str()));
        assert_eq!(count_replies(&storage, &msg.id), 1);
    }

    #[test]
    fn duplicate_event_id_replays_terminal_card_without_second_reply() {
        let (storage, human_addr) = setup();
        let msg = send_approval(&storage, &human_addr);
        let data = card_event(&msg.id, 1, "ou_user");
        let first = decide_card_action(&storage, "ou_user", &data, Some("evt-1"));
        assert!(matches!(first.decision, CardDecision::TerminalCard(_)));
        assert!(first.reply.is_some(), "首次执行应产生新回复");
        let second = decide_card_action(&storage, "ou_user", &data, Some("evt-1"));
        assert!(
            matches!(second.decision, CardDecision::TerminalCard(_)),
            "重推应回放终态卡片"
        );
        assert!(second.reply.is_none(), "幂等回放不得再次唤醒接收方");
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
                choice: Some("批准"),
                surface: "popup",
                external_event_id: None,
            },
        )
        .unwrap();
        let data = card_event(&msg.id, 0, "ou_user");
        let out = decide_card_action(&storage, "ou_user", &data, Some("evt-2"));
        match out.decision {
            CardDecision::TerminalCard(card) => {
                let text = card["body"]["elements"][2]["content"].as_str().unwrap();
                assert!(text.contains("已由 popup 处理"), "{}", text);
            }
            CardDecision::Ack => panic!("expected terminal card"),
        }
        assert!(out.reply.is_none(), "仲裁落败不产生新回复");
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
        assert!(out.reply.is_none());
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
        assert!(out.reply.is_none());
    }

    #[test]
    fn unknown_message_is_acked() {
        let (storage, _human_addr) = setup();
        let data = card_event("no-such-uuid", 0, "ou_user");
        let out = decide_card_action(&storage, "ou_user", &data, Some("evt-5"));
        assert!(matches!(out.decision, CardDecision::Ack));
        assert!(out.reply.is_none());
    }

    #[test]
    fn decide_message_bound_vs_unbound() {
        let data = json!({ "sender": { "sender_id": { "open_id": "ou_user" } } });
        assert_eq!(
            decide_message("ou_user", &data),
            Some("ou_user".to_string())
        );
        assert_eq!(decide_message("ou_other", &data), None);
    }

    #[test]
    fn link_status_defaults_disconnected() {
        let link = LinkStatus::default();
        assert!(!link.is_connected());
    }
}
