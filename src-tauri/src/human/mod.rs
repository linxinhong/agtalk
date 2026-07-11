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
    #[error("重复事件: surface={surface}, external_event_id={external_event_id}")]
    DuplicateEvent {
        surface: String,
        external_event_id: String,
    },
    #[error("JSON 错误: {0}")]
    Json(#[from] serde_json::Error),
    #[error("事件 ID 分配失败")]
    EventIdAllocation,
    #[error("目标 agent 不存在或已离开: {0}")]
    AgentNotFound(String),
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
}
