//! human 领域模块：human mailbox 的 surface fanout、审批仲裁、回复与主动发信。
//!
//! 设计见 docs/design.md §3.6。daemon 是唯一真相来源：所有 human surface
//! （popup / GUI / 浏览器扩展 / 飞书 / Android）都经 daemon 投递与回执，
//! 不允许直写 SQLite。

pub mod approval;
pub mod client;
pub mod delivery;
pub mod popup;

use crate::config::HumanConfig;
use crate::routing::{Message, SendRequest, StatusChange};
use crate::storage::Storage;
use rusqlite::OptionalExtension;
use thiserror::Error;
use uuid::Uuid;

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
    #[error("路由错误: {0}")]
    Routing(#[from] crate::routing::RoutingError),
    #[error("事件 ID 分配失败")]
    EventIdAllocation,
    #[error("目标 agent 不存在或已离开: {0}")]
    AgentNotFound(String),
    #[error("receipt 缺少结果消息 id（历史占位数据），无法确定原动作是否已落库: surface={surface} event={event}")]
    ReceiptInconclusive { surface: String, event: String },
    #[error("单选审批只能选择一个选项，实际传入 {0} 个")]
    SingleChoiceOnly(usize),
}

/// 查询 receipt 中记录的结果消息 id；缺失（历史占位数据）返回 ReceiptInconclusive。
fn receipt_message_id(
    conn: &rusqlite::Connection,
    surface: &str,
    external_event_id: &str,
) -> Result<String, HumanError> {
    let message_id: Option<String> = conn
        .query_row(
            "SELECT message_id FROM human_action_receipts \
             WHERE surface = ?1 AND external_event_id = ?2",
            rusqlite::params![surface, external_event_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    message_id.ok_or_else(|| HumanError::ReceiptInconclusive {
        surface: surface.to_string(),
        event: external_event_id.to_string(),
    })
}

fn query_message(conn: &rusqlite::Connection, id: &str) -> Result<Option<Message>, HumanError> {
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

/// human 主动发信（跨端事件幂等）：receipt 与消息插入在同一事务内原子提交。
///
/// 预生成 message UUID 直接写入 receipt——receipt 存在 ⟺ 消息存在，
/// 消除"占位 receipt + 事后补写"的崩溃窗口（窗口内重试会产生重复消息）。
/// 重复事件回放首次执行的结果。返回 (消息, 是否重复回放)。
pub fn send_with_receipt(
    storage: &Storage,
    req: &SendRequest<'_>,
    surface: &str,
    external_event_id: &str,
) -> Result<(Message, bool), HumanError> {
    let mut conn = storage.conn();
    let tx = conn.transaction()?;
    let message_id = Uuid::new_v4().to_string();
    if !delivery::record_receipt(&tx, surface, external_event_id, Some(&message_id), "send")? {
        // 重复事件：原消息与 receipt 同事务提交，必然存在，直接回放
        let original_id = receipt_message_id(&tx, surface, external_event_id)?;
        let msg =
            query_message(&tx, &original_id)?.ok_or(HumanError::MessageNotFound(original_id))?;
        return Ok((msg, true));
    }
    let msg = crate::routing::send::insert_message(&tx, &message_id, req)?;
    tx.commit()?;
    Ok((msg, false))
}

/// human 标记完成（跨端事件幂等）：receipt 与状态推进在同一事务内原子提交。
///
/// 语义与 routing::inbox::mark_done 一致：非 done → done 返回 Some(StatusChange)，
/// 已 done 返回 Ok(None)，消息不存在返回 MessageNotFound（receipt 随事务回滚，
/// 外部可修正后重试同一事件）。返回 (状态变化, 是否重复事件回放)。
pub fn done_with_receipt(
    storage: &Storage,
    message_id: &str,
    human_address: &str,
    surface: &str,
    external_event_id: &str,
) -> Result<(Option<StatusChange>, bool), HumanError> {
    let mut conn = storage.conn();
    let tx = conn.transaction()?;
    if !delivery::record_receipt(&tx, surface, external_event_id, Some(message_id), "done")? {
        // 重复事件：首次已完成，回放即可
        let _ = receipt_message_id(&tx, surface, external_event_id)?;
        return Ok((None, true));
    }
    let old_status: Option<String> = tx
        .query_row(
            "SELECT status FROM messages WHERE id = ?1 AND to_address = ?2 AND status != 'done'",
            [message_id, human_address],
            |r| r.get(0),
        )
        .optional()?;
    let old_status = match old_status {
        Some(s) => s,
        None => {
            let exists: bool = tx
                .query_row(
                    "SELECT 1 FROM messages WHERE id = ?1 AND to_address = ?2",
                    [message_id, human_address],
                    |_| Ok(true),
                )
                .optional()?
                .unwrap_or(false);
            if !exists {
                return Err(HumanError::MessageNotFound(message_id.to_string()));
            }
            tx.commit()?;
            return Ok((None, false));
        }
    };
    tx.execute(
        "UPDATE messages SET status = 'done' WHERE id = ?1 AND to_address = ?2 AND status != 'done'",
        [message_id, human_address],
    )?;
    tx.commit()?;
    Ok((
        Some(StatusChange {
            message_id: message_id.to_string(),
            old_status,
            new_status: "done".to_string(),
        }),
        false,
    ))
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

/// reconciliation：为发往 human 的未处理消息补齐当前启用 surface 的 delivery 行。
///
/// fanout 在消息提交之后执行，极端情况下可能失败/缺行；reconcile 按
/// `to_address = human` 扫描 + `INSERT OR IGNORE`，保证 delivery 可恢复。
/// 只覆盖仍需处理的消息（status 为 pending/delivered）：read/done 是历史消息，
/// 不应在 migration 后或 daemon 重启时被重新投递打扰 human。
/// 在 daemon 启动时与 fanout 失败补偿时稳定执行。返回补齐的行数。
pub fn reconcile(storage: &Storage, human_cfg: &HumanConfig) -> Result<usize, HumanError> {
    let conn = storage.conn();
    let mut inserted = 0;
    for surface in &human_cfg.surfaces {
        inserted += conn.execute(
            "INSERT OR IGNORE INTO human_deliveries (message_id, surface, status) \
             SELECT m.id, ?1, 'pending' FROM messages m \
             WHERE m.to_address = (SELECT address FROM system_mailboxes WHERE role = 'human') \
               AND m.status IN ('pending', 'delivered')",
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

    fn human_send_req<'a>(human_addr: &'a str, agent_addr: &'a str) -> SendRequest<'a> {
        SendRequest {
            to: agent_addr,
            to_name: "agent",
            from: human_addr,
            from_name: "human",
            body: "human says hi",
            content_type: "text",
            reply_to_id: None,
            subject: None,
            metadata: "{}",
            more_coming: false,
        }
    }

    fn receipt_count(storage: &Storage, surface: &str, eid: &str) -> i64 {
        let conn = storage.conn();
        conn.query_row(
            "SELECT COUNT(*) FROM human_action_receipts \
             WHERE surface = ?1 AND external_event_id = ?2",
            rusqlite::params![surface, eid],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn send_with_receipt_fresh_then_duplicate_replays() {
        let (storage, human_addr) = setup();
        let agent_addr = crate::identity::mailbox::create(&storage, "agent", "", "").unwrap();

        let (msg, dup) = send_with_receipt(
            &storage,
            &human_send_req(&human_addr, &agent_addr),
            "feishu",
            "evt-1",
        )
        .unwrap();
        assert!(!dup);

        // 重复事件：回放首次结果，不产生第二条消息
        let (replayed, dup) = send_with_receipt(
            &storage,
            &human_send_req(&human_addr, &agent_addr),
            "feishu",
            "evt-1",
        )
        .unwrap();
        assert!(dup);
        assert_eq!(replayed.id, msg.id);

        let inbox = crate::routing::inbox::inbox(&storage, &agent_addr, true).unwrap();
        assert_eq!(inbox.len(), 1);
    }

    #[test]
    fn send_with_receipt_crash_window_rolls_back_receipt() {
        let (storage, human_addr) = setup();
        let agent_addr = crate::identity::mailbox::create(&storage, "agent", "", "").unwrap();

        // 模拟事务内失败：删掉目标地址的 event_sequences 行，
        // insert_message 分配 event_id 必然失败
        {
            let conn = storage.conn();
            conn.execute(
                "DELETE FROM event_sequences WHERE address = ?1",
                [&agent_addr],
            )
            .unwrap();
        }
        let result = send_with_receipt(
            &storage,
            &human_send_req(&human_addr, &agent_addr),
            "feishu",
            "evt-2",
        );
        assert!(result.is_err());
        // 同事务回滚：receipt 无残留（旧三段式门控会留下 NULL 占位）
        assert_eq!(receipt_count(&storage, "feishu", "evt-2"), 0);

        // 修复后同一事件可重试并成功
        {
            let conn = storage.conn();
            conn.execute(
                "INSERT INTO event_sequences (address, last_event_id) VALUES (?1, 0)",
                [&agent_addr],
            )
            .unwrap();
        }
        let (msg, dup) = send_with_receipt(
            &storage,
            &human_send_req(&human_addr, &agent_addr),
            "feishu",
            "evt-2",
        )
        .unwrap();
        assert!(!dup);
        // 重试成功后再次重复：正常回放
        let (replayed, dup) = send_with_receipt(
            &storage,
            &human_send_req(&human_addr, &agent_addr),
            "feishu",
            "evt-2",
        )
        .unwrap();
        assert!(dup);
        assert_eq!(replayed.id, msg.id);
    }

    #[test]
    fn done_with_receipt_fresh_then_duplicate_replays() {
        let (storage, human_addr) = setup();
        let msg = send_to_human(&storage, &human_addr);

        let (change, dup) =
            done_with_receipt(&storage, &msg.id, &human_addr, "android", "cmd-1").unwrap();
        assert!(!dup);
        let change = change.expect("pending -> done should produce status change");
        assert_eq!(change.old_status, "pending");
        assert_eq!(change.new_status, "done");

        // 重复事件：回放，不再产生状态变化
        let (change, dup) =
            done_with_receipt(&storage, &msg.id, &human_addr, "android", "cmd-1").unwrap();
        assert!(dup);
        assert!(change.is_none());
    }

    #[test]
    fn done_with_receipt_unknown_message_rolls_back_receipt() {
        let (storage, human_addr) = setup();
        match done_with_receipt(&storage, "no-such-msg", &human_addr, "android", "cmd-2") {
            Err(HumanError::MessageNotFound(_)) => {}
            other => panic!("expected MessageNotFound, got {:?}", other.is_ok()),
        }
        // 事务回滚：receipt 无残留，修正后可重试同一事件
        assert_eq!(receipt_count(&storage, "android", "cmd-2"), 0);
    }

    #[test]
    fn reconcile_skips_done_history() {
        let (storage, human_addr) = setup();
        // 一条 pending 消息 + 一条已 done 的历史消息
        let pending_msg = send_to_human(&storage, &human_addr);
        let done_msg = send_to_human(&storage, &human_addr);
        crate::routing::inbox::mark_done(&storage, &done_msg.id, &human_addr).unwrap();

        let cfg = HumanConfig::default();
        // 只给 pending 消息补 delivery，done 历史不 backfill（不重新打扰 human）
        assert_eq!(reconcile(&storage, &cfg).unwrap(), 1);
        assert_eq!(
            delivery::list_for_message(&storage, &pending_msg.id)
                .unwrap()
                .len(),
            1
        );
        assert!(delivery::list_for_message(&storage, &done_msg.id)
            .unwrap()
            .is_empty());
    }
}
