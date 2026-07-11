//! human 领域模块：human mailbox 的 surface fanout、审批仲裁、回复与主动发信。
//!
//! 设计见 docs/design.md §3.6。daemon 是唯一真相来源：所有 human surface
//! （popup / GUI / 浏览器扩展 / 飞书 / Android）都经 daemon 投递与回执，
//! 不允许直写 SQLite。

pub mod approval;
pub mod delivery;

use crate::config::HumanConfig;
use crate::routing::Message;
use crate::storage::Storage;
use rusqlite::OptionalExtension;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HumanError {
    #[error("数据库错误: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("存储层错误: {0}")]
    Storage(#[from] crate::storage::StorageError),
    #[error("身份错误: {0}")]
    Identity(#[from] crate::identity::IdentityError),
    #[error("human mailbox 不存在")]
    HumanMailboxMissing,
    #[error("消息不存在: {0}")]
    MessageNotFound(String),
    #[error("审批已被 {resolved_by} 处理（response: {response_id}）")]
    AlreadyResolved {
        resolved_by: String,
        response_id: String,
    },
    #[error("select_only 审批必须提供 choice，不允许自由文本")]
    SelectOnlyRequiresChoice,
    #[error("无效 choice: {0}（不在审批选项中）")]
    InvalidChoice(String),
    #[error("JSON 错误: {0}")]
    Json(#[from] serde_json::Error),
    #[error("事件 ID 分配失败")]
    EventIdAllocation,
    #[error("目标 agent 不存在或已离开: {0}")]
    AgentNotFound(String),
}

/// receipt 门控结果：跨端事件（飞书回执、Android command）按 (surface, external_event_id) 去重。
pub enum ReceiptGate {
    /// 首次见到该事件：已插入占位 receipt（message_id 为 NULL），
    /// 动作成功后须 `receipt_complete` 写回结果消息 id，失败可 `receipt_abort` 允许重试。
    Fresh,
    /// 重复事件：携带原动作的结果消息 id，调用方应直接返回该结果，不再执行动作。
    Duplicate(String),
}

/// receipt 门控：占位 + 去重。重复且上次已完成时返回 `Duplicate(原结果 id)`；
/// 重复但上次未完成（占位 NULL，如上次进程中途崩溃）时删除占位并返回 `Fresh` 允许重试。
pub fn receipt_gate(
    storage: &Storage,
    surface: &str,
    external_event_id: &str,
    action: &str,
) -> Result<ReceiptGate, HumanError> {
    let conn = storage.conn();
    if delivery::record_receipt(&conn, surface, external_event_id, None, action)? {
        return Ok(ReceiptGate::Fresh);
    }
    let message_id: Option<String> = conn
        .query_row(
            "SELECT message_id FROM human_action_receipts \
             WHERE surface = ?1 AND external_event_id = ?2",
            rusqlite::params![surface, external_event_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    match message_id {
        Some(id) => Ok(ReceiptGate::Duplicate(id)),
        None => {
            conn.execute(
                "DELETE FROM human_action_receipts \
                 WHERE surface = ?1 AND external_event_id = ?2",
                rusqlite::params![surface, external_event_id],
            )?;
            delivery::record_receipt(&conn, surface, external_event_id, None, action)?;
            Ok(ReceiptGate::Fresh)
        }
    }
}

/// 动作成功后写回结果消息 id，使后续重复事件能返回原结果。
pub fn receipt_complete(
    storage: &Storage,
    surface: &str,
    external_event_id: &str,
    message_id: &str,
) -> Result<(), HumanError> {
    let conn = storage.conn();
    conn.execute(
        "UPDATE human_action_receipts SET message_id = ?3 \
         WHERE surface = ?1 AND external_event_id = ?2",
        rusqlite::params![surface, external_event_id, message_id],
    )?;
    Ok(())
}

/// 动作失败时删除占位，允许外部重试同一事件。
pub fn receipt_abort(
    storage: &Storage,
    surface: &str,
    external_event_id: &str,
) -> Result<(), HumanError> {
    let conn = storage.conn();
    conn.execute(
        "DELETE FROM human_action_receipts WHERE surface = ?1 AND external_event_id = ?2",
        rusqlite::params![surface, external_event_id],
    )?;
    Ok(())
}

/// 查询 human mailbox 地址（system_mailboxes role='human'）。
pub fn human_address(storage: &Storage) -> Result<String, HumanError> {
    let conn = storage.conn();
    let addr: Option<String> = conn
        .query_row(
            "SELECT address FROM system_mailboxes WHERE role = 'human'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    addr.ok_or(HumanError::HumanMailboxMissing)
}

/// fanout：消息发往 human 后，为每个配置的 surface 记一条 pending delivery。
///
/// `INSERT OR IGNORE` 保证幂等（同一 message+surface 只投递一次）。
/// phase-1 只做持久化记账：surface 拉取/订阅 SSE 拿到消息后回执 delivered。
/// 返回新写入的 delivery 条数。
pub fn fanout(
    storage: &Storage,
    human_cfg: &HumanConfig,
    msg: &Message,
) -> Result<usize, HumanError> {
    let conn = storage.conn();
    let mut inserted = 0;
    for surface in &human_cfg.surfaces {
        inserted += conn.execute(
            "INSERT OR IGNORE INTO human_deliveries (message_id, surface, status) \
             VALUES (?1, ?2, 'pending')",
            rusqlite::params![msg.id, surface],
        )?;
    }
    Ok(inserted)
}

/// reconciliation：为所有发往 human 的消息补齐当前启用 surface 的 delivery 行。
///
/// fanout 在消息提交之后执行，极端情况下可能失败/缺行；reconcile 按
/// `to_address = human` 全量扫描 + `INSERT OR IGNORE`，保证 delivery 可恢复。
/// 在 daemon 启动时与 fanout 失败补偿时稳定执行。返回补齐的行数。
pub fn reconcile(storage: &Storage, human_cfg: &HumanConfig) -> Result<usize, HumanError> {
    let conn = storage.conn();
    let mut inserted = 0;
    for surface in &human_cfg.surfaces {
        inserted += conn.execute(
            "INSERT OR IGNORE INTO human_deliveries (message_id, surface, status) \
             SELECT m.id, ?1, 'pending' FROM messages m \
             WHERE m.to_address = (SELECT address FROM system_mailboxes WHERE role = 'human')",
            [surface],
        )?;
    }
    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::mailbox::ensure_human;
    use crate::routing::{send::send, SendRequest};

    fn setup() -> (Storage, String) {
        let storage = Storage::open_in_memory().unwrap();
        let cfg = HumanConfig::default();
        let human_addr = ensure_human(&storage, &cfg).unwrap();
        (storage, human_addr)
    }

    fn send_to_human(storage: &Storage, human_addr: &str) -> Message {
        let agent_addr = crate::identity::mailbox::create(storage, "agent", "", "").unwrap();
        send(
            storage,
            SendRequest {
                to: human_addr,
                to_name: "human",
                from: &agent_addr,
                from_name: "agent",
                body: "hello human",
                content_type: "text",
                reply_to_id: None,
                subject: None,
                metadata: "{}",
                more_coming: false,
            },
        )
        .unwrap()
    }

    #[test]
    fn human_address_found_after_ensure() {
        let (storage, addr) = setup();
        assert_eq!(human_address(&storage).unwrap(), addr);
    }

    #[test]
    fn human_address_missing_without_ensure() {
        let storage = Storage::open_in_memory().unwrap();
        match human_address(&storage) {
            Err(HumanError::HumanMailboxMissing) => {}
            other => panic!("expected HumanMailboxMissing, got {:?}", other),
        }
    }

    #[test]
    fn fanout_writes_pending_for_each_surface() {
        let (storage, human_addr) = setup();
        let msg = send_to_human(&storage, &human_addr);
        let cfg = HumanConfig {
            surfaces: vec!["popup".into(), "feishu".into()],
            ..HumanConfig::default()
        };

        let n = fanout(&storage, &cfg, &msg).unwrap();
        assert_eq!(n, 2);

        let deliveries = delivery::list_for_message(&storage, &msg.id).unwrap();
        assert_eq!(deliveries.len(), 2);
        assert_eq!(deliveries[0].surface, "feishu");
        assert_eq!(deliveries[1].surface, "popup");
        assert!(deliveries.iter().all(|d| d.status == "pending"));
    }

    #[test]
    fn fanout_is_idempotent_for_same_message() {
        let (storage, human_addr) = setup();
        let msg = send_to_human(&storage, &human_addr);
        let cfg = HumanConfig::default();

        assert_eq!(fanout(&storage, &cfg, &msg).unwrap(), 1);
        assert_eq!(fanout(&storage, &cfg, &msg).unwrap(), 0);
        assert_eq!(
            delivery::list_for_message(&storage, &msg.id).unwrap().len(),
            1
        );
    }

    #[test]
    fn reconcile_backfills_missing_delivery_rows() {
        let (storage, human_addr) = setup();
        // 模拟 fanout 未执行/失败：消息落库但无 delivery 行
        let msg = send_to_human(&storage, &human_addr);
        assert!(delivery::list_for_message(&storage, &msg.id)
            .unwrap()
            .is_empty());

        let cfg = HumanConfig::default();
        assert_eq!(reconcile(&storage, &cfg).unwrap(), 1);
        let ds = delivery::list_for_message(&storage, &msg.id).unwrap();
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].status, "pending");

        // 幂等：再跑不重复补
        assert_eq!(reconcile(&storage, &cfg).unwrap(), 0);
    }

    #[test]
    fn reconcile_restores_deleted_row_and_covers_enabled_surfaces() {
        let (storage, human_addr) = setup();
        let msg = send_to_human(&storage, &human_addr);
        let cfg = HumanConfig {
            surfaces: vec!["popup".into(), "feishu".into()],
            ..HumanConfig::default()
        };
        assert_eq!(fanout(&storage, &cfg, &msg).unwrap(), 2);

        // 模拟 delivery 行丢失（fanout 失败/误删）
        {
            let conn = storage.conn();
            conn.execute(
                "DELETE FROM human_deliveries WHERE message_id = ?1 AND surface = 'feishu'",
                [&msg.id],
            )
            .unwrap();
        }
        assert_eq!(
            delivery::list_for_message(&storage, &msg.id).unwrap().len(),
            1
        );

        // reconcile 只补缺的 feishu 行，不动已有 popup 行
        assert_eq!(reconcile(&storage, &cfg).unwrap(), 1);
        let ds = delivery::list_for_message(&storage, &msg.id).unwrap();
        assert_eq!(ds.len(), 2);
        assert!(ds.iter().any(|d| d.surface == "feishu"));
    }

    #[test]
    fn receipt_gate_fresh_complete_then_duplicate() {
        let (storage, _) = setup();
        match receipt_gate(&storage, "feishu", "evt-1", "send").unwrap() {
            ReceiptGate::Fresh => {}
            _ => panic!("first event should be Fresh"),
        }
        receipt_complete(&storage, "feishu", "evt-1", "msg-42").unwrap();
        match receipt_gate(&storage, "feishu", "evt-1", "send").unwrap() {
            ReceiptGate::Duplicate(id) => assert_eq!(id, "msg-42"),
            _ => panic!("completed event should be Duplicate"),
        }
    }

    #[test]
    fn receipt_gate_abort_allows_retry() {
        let (storage, _) = setup();
        assert!(matches!(
            receipt_gate(&storage, "feishu", "evt-2", "done").unwrap(),
            ReceiptGate::Fresh
        ));
        receipt_abort(&storage, "feishu", "evt-2").unwrap();
        // abort 后同一事件重新视为首次
        assert!(matches!(
            receipt_gate(&storage, "feishu", "evt-2", "done").unwrap(),
            ReceiptGate::Fresh
        ));
    }

    #[test]
    fn receipt_gate_recovers_from_null_placeholder() {
        let (storage, _) = setup();
        // 模拟上次动作中途崩溃：占位 receipt 无结果 id
        {
            let conn = storage.conn();
            delivery::record_receipt(&conn, "android", "cmd-1", None, "send").unwrap();
        }
        // 重复事件但无结果：自动删除占位并放行
        assert!(matches!(
            receipt_gate(&storage, "android", "cmd-1", "send").unwrap(),
            ReceiptGate::Fresh
        ));
        receipt_complete(&storage, "android", "cmd-1", "msg-7").unwrap();
        match receipt_gate(&storage, "android", "cmd-1", "send").unwrap() {
            ReceiptGate::Duplicate(id) => assert_eq!(id, "msg-7"),
            _ => panic!("should be Duplicate after complete"),
        }
    }
}
