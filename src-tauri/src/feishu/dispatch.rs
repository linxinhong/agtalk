//! 出站投递与仲裁收尾：fanout 后把 feishu surface 的 pending delivery 投递到飞书，
//! human 消息被任一 surface 处理后向飞书卡片回写终态（approval 回显选项，文本回显回复）。
//!
//! 与 popup dispatch 同层：msg 发往 human 后由 handler 调用 dispatch；
//! settle 在 human reply 胜出后由 actions handler 调用。均为 fire-and-forget
//! （spawn 异步任务），失败标 failed 可被 reconcile 重试，不影响消息落库。

use super::{card, client::FeishuClient, router::SURFACE};
use crate::config::FeishuConfig;
use crate::human::delivery;
use crate::routing::Message;
use crate::storage::Storage;
use rusqlite::OptionalExtension;
use tracing::warn;

/// 飞书出站投递器：daemon 启用，测试默认 disabled（不发真实请求）。
pub enum FeishuDispatcher {
    Disabled,
    Enabled { cfg: FeishuConfig },
}

impl FeishuDispatcher {
    pub fn disabled() -> Self {
        Self::Disabled
    }

    pub fn enabled(cfg: FeishuConfig) -> Self {
        Self::Enabled { cfg }
    }

    /// 投递一条 human 消息到飞书：approval → 审批卡片，其它 → 文本卡片；
    /// send_card 失败回退纯文本，再失败标 failed（reconcile 可重试）。
    pub fn dispatch(&self, storage: &Storage, msg: &Message, surfaces: &[String]) {
        let Self::Enabled { cfg } = self else {
            return;
        };
        if !surfaces.iter().any(|s| s == SURFACE) {
            return;
        }
        if cfg.open_id.is_empty() {
            warn!("feishu surface 已配置但 open_id 未绑定，跳过投递");
            return;
        }
        let client = FeishuClient::new(&cfg.base_url, &cfg.app_id, &cfg.app_secret);
        let storage = storage.clone();
        let msg = msg.clone();
        let open_id = cfg.open_id.clone();
        tokio::spawn(async move {
            deliver_message(&storage, &client, &open_id, &msg).await;
        });
    }

    /// 仲裁/抢答收尾：human 消息被某 surface 处理后，向飞书卡片回写终态（best-effort）。
    /// approval 回显胜出选项；文本回复（reply 提供时）回显原消息 + 回复正文。
    pub fn settle(
        &self,
        storage: &Storage,
        message_id: &str,
        resolved_by: &str,
        reply: Option<&Message>,
    ) {
        let Self::Enabled { cfg } = self else {
            return;
        };
        let Some((open_message_id, card)) = plan_settle(storage, message_id, resolved_by, reply)
        else {
            return;
        };
        let client = FeishuClient::new(&cfg.base_url, &cfg.app_id, &cfg.app_secret);
        tokio::spawn(async move {
            if let Err(e) = client.patch_card(&open_message_id, &card).await {
                warn!("feishu 终态回写失败: {}", e);
            }
        });
    }
}

/// 出站投递主逻辑（可测）：卡片 → 失败回退纯文本 → 再失败标 failed。
pub async fn deliver_message(
    storage: &Storage,
    client: &FeishuClient,
    open_id: &str,
    msg: &Message,
) {
    let card = if msg.content_type == "approval_request" {
        card::approval_card(msg, &card::approval_choices(msg))
    } else {
        card::text_card(msg)
    };
    let delivered_ref = match client.send_card(open_id, &card).await {
        Ok(id) => Some(id),
        Err(e) => {
            warn!("feishu send_card 失败，回退纯文本: {}", e);
            let text = format!("[agtalk] {}: {}", msg.from_name, msg.body);
            match client.send_text(open_id, &text).await {
                Ok(id) => Some(id),
                Err(e2) => {
                    let err = e2.to_string();
                    let _ = delivery::deliver_via(storage, msg, SURFACE, move |_| {
                        Err::<Option<String>, String>(err)
                    });
                    return;
                }
            }
        }
    };
    let _ = delivery::deliver_via(storage, msg, SURFACE, move |_| Ok(delivered_ref));
}

/// 仲裁/抢答收尾决策（可测）：找到该消息已 delivered 的 feishu delivery 及其外部引用，
/// 生成终态卡片；无对应 delivery 或缺 external_ref 返回 None。
/// approval 回显胜出选项；文本消息在 reply 提供时回显原消息 + 回复正文，否则不回写。
pub fn plan_settle(
    storage: &Storage,
    message_id: &str,
    resolved_by: &str,
    reply: Option<&Message>,
) -> Option<(String, serde_json::Value)> {
    let original = {
        let conn = storage.conn();
        conn.query_row(
            "SELECT id, to_address, to_name, from_address, from_name, body, content_type, \
             reply_to_id, subject, metadata, event_id, status, created_at \
             FROM messages WHERE id = ?1",
            [message_id],
            Message::from_row,
        )
        .ok()?
    };
    let ds = delivery::list_for_message(storage, message_id).ok()?;
    let d = ds
        .iter()
        .find(|d| d.surface == SURFACE && d.status == "delivered")?;
    let open_message_id = d.external_ref.clone()?;
    let card = if original.content_type == "approval_request" {
        // 回显胜出选项（resolution 由胜出 surface 写入；查不到则只显示处理方）
        let selected: Option<String> = {
            let conn = storage.conn();
            conn.query_row(
                "SELECT selected_choice FROM approval_resolutions WHERE request_message_id = ?1",
                [message_id],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
        };
        card::terminal_card(
            &original,
            &format!("已由 {} 处理", resolved_by),
            selected.as_deref(),
        )
    } else {
        // 文本消息：抢答收尾回显原消息 + 回复正文，保留对话上下文
        let reply = reply?;
        card::reply_terminal_card(
            &original,
            &reply.body,
            &format!("已由 {} 回复", resolved_by),
        )
    };
    Some((open_message_id, card))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HumanConfig;
    use crate::human::fanout;
    use crate::identity::mailbox::{create, ensure_human};
    use crate::routing::{send::send, SendRequest};
    use crate::testutil::mock_http_server;
    use serde_json::json;

    fn setup() -> (Storage, String) {
        let storage = Storage::open_in_memory().unwrap();
        let cfg = HumanConfig {
            surfaces: vec![SURFACE.into()],
            ..HumanConfig::default()
        };
        let human_addr = ensure_human(&storage, &cfg).unwrap();
        (storage, human_addr)
    }

    fn send_approval(storage: &Storage, human_addr: &str) -> Message {
        let agent = create(storage, "agent", "", "").unwrap();
        let msg = send(
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
        .unwrap();
        fanout(
            storage,
            &HumanConfig {
                surfaces: vec![SURFACE.into()],
                ..HumanConfig::default()
            },
            &msg,
        )
        .unwrap();
        msg
    }

    fn token_ok() -> String {
        json!({ "code": 0, "msg": "ok", "tenant_access_token": "t-1", "expire": 7200 }).to_string()
    }

    fn sent_ok(message_id: &str) -> String {
        json!({ "code": 0, "msg": "ok", "data": { "message_id": message_id } }).to_string()
    }

    #[tokio::test]
    async fn deliver_approval_card_marks_delivered_with_external_ref() {
        let (storage, human_addr) = setup();
        let msg = send_approval(&storage, &human_addr);
        let (base, _rx, handle) = mock_http_server(vec![token_ok(), sent_ok("om_card_1")]);
        let client = FeishuClient::new(&base, "cli_a", "secret");
        deliver_message(&storage, &client, "ou_user", &msg).await;
        let ds = delivery::list_for_message(&storage, &msg.id).unwrap();
        assert_eq!(ds[0].status, "delivered");
        assert_eq!(ds[0].external_ref.as_deref(), Some("om_card_1"));
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn card_failure_falls_back_to_text() {
        let (storage, human_addr) = setup();
        let msg = send_approval(&storage, &human_addr);
        let card_err = json!({ "code": 999, "msg": "card rejected" }).to_string();
        let (base, _rx, handle) =
            mock_http_server(vec![token_ok(), card_err, sent_ok("om_text_1")]);
        let client = FeishuClient::new(&base, "cli_a", "secret");
        deliver_message(&storage, &client, "ou_user", &msg).await;
        let ds = delivery::list_for_message(&storage, &msg.id).unwrap();
        assert_eq!(ds[0].status, "delivered");
        assert_eq!(ds[0].external_ref.as_deref(), Some("om_text_1"));
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn both_fail_marks_failed_with_attempts() {
        let (storage, human_addr) = setup();
        let msg = send_approval(&storage, &human_addr);
        let err = json!({ "code": 999, "msg": "boom" }).to_string();
        let (base, _rx, handle) = mock_http_server(vec![token_ok(), err.clone(), err]);
        let client = FeishuClient::new(&base, "cli_a", "secret");
        deliver_message(&storage, &client, "ou_user", &msg).await;
        let ds = delivery::list_for_message(&storage, &msg.id).unwrap();
        assert_eq!(ds[0].status, "failed");
        assert_eq!(ds[0].attempts, 1);
        assert!(ds[0].error.is_some());
        handle.join().unwrap();
    }

    #[test]
    fn plan_settle_returns_terminal_card_for_delivered_feishu() {
        let (storage, human_addr) = setup();
        let msg = send_approval(&storage, &human_addr);
        delivery::mark_delivered(&storage, &msg.id, SURFACE, Some("om_settle_1")).unwrap();
        let (open_message_id, card) = plan_settle(&storage, &msg.id, "popup", None).unwrap();
        assert_eq!(open_message_id, "om_settle_1");
        let texts: Vec<&str> = card["body"]["elements"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e.get("content").and_then(|c| c.as_str()))
            .collect();
        assert!(
            texts.iter().any(|t| t.contains("已由 popup 处理")),
            "{:?}",
            texts
        );
    }

    #[test]
    fn plan_settle_none_without_delivered_feishu() {
        let (storage, human_addr) = setup();
        let msg = send_approval(&storage, &human_addr);
        // delivery 仍是 pending（未投递成功）→ 不回写
        assert!(plan_settle(&storage, &msg.id, "popup", None).is_none());
    }

    fn send_text_and_reply(storage: &Storage, human_addr: &str) -> (Message, Message) {
        let agent = create(storage, "agent", "", "").unwrap();
        let msg = send(
            storage,
            SendRequest {
                to: human_addr,
                to_name: "human",
                from: &agent,
                from_name: "agent",
                body: "进展如何？",
                content_type: "text",
                reply_to_id: None,
                subject: None,
                metadata: "{}",
                more_coming: false,
            },
        )
        .unwrap();
        fanout(
            storage,
            &HumanConfig {
                surfaces: vec![SURFACE.into()],
                ..HumanConfig::default()
            },
            &msg,
        )
        .unwrap();
        let reply = crate::human::approval::reply(
            storage,
            crate::human::approval::HumanReplyRequest {
                message_id: &msg.id,
                body: "已完成",
                choice: None,
                surface: "popup",
                external_event_id: None,
            },
        )
        .unwrap()
        .reply;
        (msg, reply)
    }

    #[test]
    fn plan_settle_text_reply_shows_original_and_reply_body() {
        let (storage, human_addr) = setup();
        let (msg, reply) = send_text_and_reply(&storage, &human_addr);
        delivery::mark_delivered(&storage, &msg.id, SURFACE, Some("om_text_settle")).unwrap();
        let (open_message_id, card) =
            plan_settle(&storage, &msg.id, "popup", Some(&reply)).unwrap();
        assert_eq!(open_message_id, "om_text_settle");
        let texts: Vec<&str> = card["body"]["elements"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e.get("content").and_then(|c| c.as_str()))
            .collect();
        // 原消息上下文 + 回复正文 + 处理方状态行
        assert!(
            texts.iter().any(|t| t.contains("进展如何？")),
            "{:?}",
            texts
        );
        assert!(texts.iter().any(|t| t.contains("已完成")), "{:?}", texts);
        assert!(
            texts.iter().any(|t| t.contains("已由 popup 回复")),
            "{:?}",
            texts
        );
    }

    #[test]
    fn plan_settle_text_without_reply_returns_none() {
        let (storage, human_addr) = setup();
        let (msg, _reply) = send_text_and_reply(&storage, &human_addr);
        delivery::mark_delivered(&storage, &msg.id, SURFACE, Some("om_text_2")).unwrap();
        // 文本消息没有回复正文可回显 → 不回写
        assert!(plan_settle(&storage, &msg.id, "popup", None).is_none());
    }

    #[test]
    fn disabled_dispatcher_is_noop() {
        let (storage, human_addr) = setup();
        let msg = send_approval(&storage, &human_addr);
        let d = FeishuDispatcher::disabled();
        d.dispatch(&storage, &msg, &[SURFACE.to_string()]);
        d.settle(&storage, &msg.id, "popup", None);
        let ds = delivery::list_for_message(&storage, &msg.id).unwrap();
        assert_eq!(ds[0].status, "pending");
    }
}
