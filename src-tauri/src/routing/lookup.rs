//! lookup：按 name 查询候选 mailbox，以及消息详情。

use super::{Message, RoutingError};
use crate::identity::mailbox;
use crate::storage::Storage;
use rusqlite::OptionalExtension;

pub fn lookup(
    storage: &Storage,
    name_filter: Option<&str>,
) -> Result<Vec<mailbox::Mailbox>, RoutingError> {
    let mbs = mailbox::list(storage, name_filter)?;
    Ok(mbs)
}

/// 查询单条消息详情（不标记已读）。
pub fn detail(storage: &Storage, message_id: &str) -> Result<Option<Message>, RoutingError> {
    let conn = storage.conn();
    let mut stmt = conn.prepare("SELECT * FROM messages WHERE id = ?1")?;
    let msg = stmt.query_row([message_id], Message::from_row).optional()?;
    Ok(msg)
}

/// 查询消息详情并标记已读；`detail -` 语义：先最新未读，没有再最新一条。
pub fn detail_and_mark_read(
    storage: &Storage,
    address: &str,
    message_id: &str,
) -> Result<Option<Message>, RoutingError> {
    let conn = storage.conn();
    let msg: Option<Message> = if message_id == "-" {
        conn.query_row(
            "SELECT * FROM messages WHERE to_address = ?1 AND status != 'done' \
             ORDER BY event_id DESC LIMIT 1",
            [address],
            Message::from_row,
        )
        .optional()?
        .or_else(|| {
            conn.query_row(
                "SELECT * FROM messages WHERE to_address = ?1 ORDER BY event_id DESC LIMIT 1",
                [address],
                Message::from_row,
            )
            .optional()
            .unwrap_or(None)
        })
    } else {
        let mut stmt = conn.prepare("SELECT * FROM messages WHERE id = ?1")?;
        stmt.query_row([message_id], Message::from_row).optional()?
    };

    if let Some(ref m) = msg {
        let _ = conn.execute(
            "UPDATE messages SET status = 'read' WHERE id = ?1 AND status IN ('pending', 'delivered')",
            [&m.id],
        );
    }

    Ok(msg)
}
