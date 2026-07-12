//! human_deliveries 状态机与 human_action_receipts 去重。
//!
//! delivery 状态：pending → delivered（surface 确认收到）/ failed（可重试）。
//! receipts 用于跨端事件去重（飞书消息回执、Android command 等）：同一
//! (surface, external_event_id) 只处理一次。

use super::HumanError;
use crate::routing::Message;
use crate::storage::Storage;
use rusqlite::OptionalExtension;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    pub message_id: String,
    pub surface: String,
    pub status: String,
    pub attempts: i64,
    pub external_ref: Option<String>,
    pub error: Option<String>,
}

fn row_to_delivery(row: &rusqlite::Row<'_>) -> Result<Delivery, rusqlite::Error> {
    Ok(Delivery {
        message_id: row.get(0)?,
        surface: row.get(1)?,
        status: row.get(2)?,
        attempts: row.get(3)?,
        external_ref: row.get(4)?,
        error: row.get(5)?,
    })
}

fn query_deliveries(
    conn: &rusqlite::Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Result<Vec<Delivery>, HumanError> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, row_to_delivery)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 查询某消息的所有 delivery（按 surface 排序，输出稳定）。
pub fn list_for_message(storage: &Storage, message_id: &str) -> Result<Vec<Delivery>, HumanError> {
    let conn = storage.conn();
    query_deliveries(
        &conn,
        "SELECT message_id, surface, status, attempts, external_ref, error \
         FROM human_deliveries WHERE message_id = ?1 ORDER BY surface",
        &[&message_id],
    )
}

/// 查询某 surface 下可投递/可重试的 delivery（pending 或 failed）。
pub fn pending_for_surface(storage: &Storage, surface: &str) -> Result<Vec<Delivery>, HumanError> {
    let conn = storage.conn();
    query_deliveries(
        &conn,
        "SELECT message_id, surface, status, attempts, external_ref, error \
         FROM human_deliveries WHERE surface = ?1 AND status IN ('pending', 'failed') \
         ORDER BY created_at",
        &[&surface],
    )
}

/// 标记 delivery 已送达（surface 回执）。external_ref 提供时覆盖原值，error 清零。
pub fn mark_delivered(
    storage: &Storage,
    message_id: &str,
    surface: &str,
    external_ref: Option<&str>,
) -> Result<(), HumanError> {
    let conn = storage.conn();
    conn.execute(
        "UPDATE human_deliveries \
         SET status = 'delivered', external_ref = COALESCE(?3, external_ref), error = NULL, \
             updated_at = unixepoch('subsec') \
         WHERE message_id = ?1 AND surface = ?2",
        rusqlite::params![message_id, surface, external_ref],
    )?;
    Ok(())
}

/// surface 确认已展示（delivery 回执）：标 delivered、error 清零。
/// 幂等：已 delivered 再 ack 仍返回 true。返回 false 表示该 delivery 行不存在。
pub fn ack(storage: &Storage, message_id: &str, surface: &str) -> Result<bool, HumanError> {
    let conn = storage.conn();
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM human_deliveries WHERE message_id = ?1 AND surface = ?2",
            rusqlite::params![message_id, surface],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if !exists {
        return Ok(false);
    }
    conn.execute(
        "UPDATE human_deliveries \
         SET status = 'delivered', error = NULL, updated_at = unixepoch('subsec') \
         WHERE message_id = ?1 AND surface = ?2",
        rusqlite::params![message_id, surface],
    )?;
    Ok(true)
}

/// 记录 receipt 去重；返回 true 表示首次见到该事件。
///
/// 只操作传入的 connection，调用方若在事务内使用可保证与后续动作原子
/// （审批路径在 approval.rs 单事务内调用）。
pub fn record_receipt(
    conn: &rusqlite::Connection,
    surface: &str,
    external_event_id: &str,
    message_id: Option<&str>,
    action: &str,
) -> Result<bool, HumanError> {
    let changed = conn.execute(
        "INSERT OR IGNORE INTO human_action_receipts \
         (surface, external_event_id, message_id, action) \
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![surface, external_event_id, message_id, action],
    )?;
    Ok(changed > 0)
}

/// 执行一次 delivery：调用 transport 闭包，成功标 delivered，失败标 failed 并累加 attempts。
///
/// transport 返回 `Ok(Some(ref))` 携带外部引用（如飞书 message_id），`Err(e)` 的错误字符串
/// 会持久化，便于 doctor / UI 展示与后续重试。delivery 记账本身失败才返回 Err。
pub fn deliver_via<F>(
    storage: &Storage,
    message: &Message,
    surface: &str,
    transport: F,
) -> Result<(), HumanError>
where
    F: FnOnce(&Message) -> Result<Option<String>, String>,
{
    match transport(message) {
        Ok(external_ref) => mark_delivered(storage, &message.id, surface, external_ref.as_deref()),
        Err(err) => {
            let conn = storage.conn();
            conn.execute(
                "UPDATE human_deliveries \
                 SET status = 'failed', attempts = attempts + 1, error = ?3, \
                     updated_at = unixepoch('subsec') \
                 WHERE message_id = ?1 AND surface = ?2",
                rusqlite::params![message.id, surface, err],
            )?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HumanConfig;
    use crate::human::{fanout, human_address};
    use crate::identity::mailbox::ensure_human;
    use crate::routing::{send::send, SendRequest};

    fn setup_with_message() -> (Storage, Message) {
        let storage = Storage::open_in_memory().unwrap();
        let cfg = HumanConfig {
            surfaces: vec!["popup".into(), "feishu".into()],
            ..HumanConfig::default()
        };
        let human_addr = ensure_human(&storage, &cfg).unwrap();
        let agent_addr = crate::identity::mailbox::create(&storage, "agent", "", "").unwrap();
        let msg = send(
            &storage,
            SendRequest {
                to: &human_addr,
                to_name: "human",
                from: &agent_addr,
                from_name: "agent",
                body: "please approve",
                content_type: "approval_request",
                reply_to_id: None,
                subject: None,
                metadata: "{}",
                more_coming: false,
            },
        )
        .unwrap();
        fanout(&storage, &cfg, &msg).unwrap();
        (storage, msg)
    }

    #[test]
    fn deliver_via_success_marks_delivered_with_ref() {
        let (storage, msg) = setup_with_message();
        deliver_via(&storage, &msg, "popup", |m| {
            assert_eq!(m.id, msg.id);
            Ok(Some("popup-window-1".to_string()))
        })
        .unwrap();

        let ds = list_for_message(&storage, &msg.id).unwrap();
        let popup = ds.iter().find(|d| d.surface == "popup").unwrap();
        assert_eq!(popup.status, "delivered");
        assert_eq!(popup.external_ref.as_deref(), Some("popup-window-1"));
        assert_eq!(popup.error, None);

        // delivered 后不再出现在 pending 列表
        let pending = pending_for_surface(&storage, "popup").unwrap();
        assert!(pending.is_empty());
        // feishu 仍 pending
        assert_eq!(pending_for_surface(&storage, "feishu").unwrap().len(), 1);
    }

    #[test]
    fn deliver_via_failure_marks_failed_and_retry_succeeds() {
        let (storage, msg) = setup_with_message();

        deliver_via(
            &storage,
            &msg,
            "feishu",
            |_| Err("open_id 无效".to_string()),
        )
        .unwrap();
        let ds = list_for_message(&storage, &msg.id).unwrap();
        let feishu = ds.iter().find(|d| d.surface == "feishu").unwrap();
        assert_eq!(feishu.status, "failed");
        assert_eq!(feishu.attempts, 1);
        assert_eq!(feishu.error.as_deref(), Some("open_id 无效"));

        // failed 仍可被重试捞取
        assert_eq!(pending_for_surface(&storage, "feishu").unwrap().len(), 1);

        // 重试成功：状态回 delivered，error 清零
        deliver_via(&storage, &msg, "feishu", |_| Ok(Some("om-123".to_string()))).unwrap();
        let ds = list_for_message(&storage, &msg.id).unwrap();
        let feishu = ds.iter().find(|d| d.surface == "feishu").unwrap();
        assert_eq!(feishu.status, "delivered");
        assert_eq!(feishu.external_ref.as_deref(), Some("om-123"));
        assert_eq!(feishu.error, None);
    }

    #[test]
    fn ack_marks_delivered_and_is_idempotent() {
        let (storage, msg) = setup_with_message();
        // 不存在的 delivery 行返回 false
        assert!(!ack(&storage, &msg.id, "wechat").unwrap());
        assert!(!ack(&storage, "no-such-msg", "popup").unwrap());

        assert!(ack(&storage, &msg.id, "popup").unwrap());
        let ds = list_for_message(&storage, &msg.id).unwrap();
        let popup = ds.iter().find(|d| d.surface == "popup").unwrap();
        assert_eq!(popup.status, "delivered");

        // 幂等：再次 ack 仍 true
        assert!(ack(&storage, &msg.id, "popup").unwrap());
    }

    #[test]
    fn record_receipt_dedups_same_surface_event() {
        let storage = Storage::open_in_memory().unwrap();
        let conn = storage.conn();

        assert!(record_receipt(&conn, "feishu", "evt-1", Some("m1"), "reply").unwrap());
        // 同 surface + 同事件：重复，不再处理
        assert!(!record_receipt(&conn, "feishu", "evt-1", Some("m1"), "reply").unwrap());
        // 不同 surface 的同一外部事件 id 互不影响
        assert!(record_receipt(&conn, "android", "evt-1", Some("m1"), "reply").unwrap());
        // 同 surface 不同事件正常记录
        assert!(record_receipt(&conn, "feishu", "evt-2", None, "done").unwrap());
    }

    #[test]
    fn human_address_roundtrip_in_delivery_module() {
        let storage = Storage::open_in_memory().unwrap();
        let cfg = HumanConfig::default();
        let addr = ensure_human(&storage, &cfg).unwrap();
        assert_eq!(human_address(&storage).unwrap(), addr);
    }
}
