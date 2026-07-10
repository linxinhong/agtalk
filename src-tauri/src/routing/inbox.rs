//! inbox：一次性查询收件箱。

use super::{Message, RoutingError, StatusChange};
use crate::storage::Storage;
use rusqlite::params;
use rusqlite::OptionalExtension;

/// 按 event_id 顺序重放某地址 event_id > after 的消息（SSE 断线续传）。
pub fn events_since(
    storage: &Storage,
    address: &str,
    after_event_id: i64,
) -> Result<Vec<Message>, RoutingError> {
    let conn = storage.conn();
    let mut stmt = conn.prepare(
        "SELECT * FROM messages WHERE to_address = ?1 AND event_id > ?2 ORDER BY event_id",
    )?;
    let mapped = stmt.query_map(params![address, after_event_id], super::Message::from_row)?;
    Ok(mapped.collect::<Result<_, _>>()?)
}

/// 返回某地址当前最大 event_id；无消息时返回 0。
pub fn max_event_id(storage: &Storage, address: &str) -> Result<i64, RoutingError> {
    let conn = storage.conn();
    let id: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(event_id), 0) FROM messages WHERE to_address = ?1",
            [address],
            |row| row.get(0),
        )
        .unwrap_or(0);
    Ok(id)
}

/// 返回 status 为 pending 或 delivered 的消息（即真正未读）。
pub fn unread_inbox(storage: &Storage, address: &str) -> Result<Vec<Message>, RoutingError> {
    let conn = storage.conn();
    let mut stmt = conn.prepare(
        "SELECT * FROM messages WHERE to_address = ?1 AND status IN ('pending', 'delivered') ORDER BY event_id DESC",
    )?;
    let mapped = stmt.query_map([address], super::Message::from_row)?;
    Ok(mapped.collect::<Result<_, _>>()?)
}

pub fn inbox(
    storage: &Storage,
    address: &str,
    include_done: bool,
) -> Result<Vec<Message>, RoutingError> {
    let conn = storage.conn();
    let (sql, params): (&str, Vec<&dyn rusqlite::ToSql>) = if include_done {
        (
            "SELECT * FROM messages WHERE to_address = ?1 ORDER BY event_id DESC",
            vec![&address as &dyn rusqlite::ToSql],
        )
    } else {
        (
            "SELECT * FROM messages WHERE to_address = ?1 AND status != 'done' ORDER BY event_id DESC",
            vec![&address as &dyn rusqlite::ToSql],
        )
    };

    let mut stmt = conn.prepare(sql)?;
    let mapped = stmt.query_map(params.as_slice(), super::Message::from_row)?;
    Ok(mapped.collect::<Result<_, _>>()?)
}

/// 统计状态为 pending 的消息数量。
pub fn count_pending(storage: &Storage) -> Result<i64, RoutingError> {
    let conn = storage.conn();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    Ok(count)
}

/// 将消息状态标记为 delivered（SSE 推送后调用）。
pub fn mark_delivered(storage: &Storage, message_id: &str) -> Result<(), RoutingError> {
    let conn = storage.conn();
    conn.execute(
        "UPDATE messages SET status = 'delivered' WHERE id = ?1 AND status = 'pending'",
        [message_id],
    )?;
    Ok(())
}

/// 将消息状态标记为 read。
/// 只有 pending/delivered -> read 才返回 Some(StatusChange)。
pub fn mark_read(
    storage: &Storage,
    message_id: &str,
) -> Result<Option<StatusChange>, RoutingError> {
    let conn = storage.conn();
    let old_status: Option<String> = conn
        .query_row(
            "SELECT status FROM messages WHERE id = ?1 AND status IN ('pending', 'delivered')",
            [message_id],
            |row| row.get(0),
        )
        .optional()?;
    let old_status = match old_status {
        Some(s) => s,
        None => return Ok(None),
    };
    conn.execute(
        "UPDATE messages SET status = 'read' WHERE id = ?1 AND status IN ('pending', 'delivered')",
        [message_id],
    )?;
    Ok(Some(super::StatusChange {
        message_id: message_id.to_string(),
        old_status,
        new_status: "read".to_string(),
    }))
}

/// 将消息状态标记为 done；验证该消息确实属于指定 address。
/// 只有非 done -> done 才返回 Some(StatusChange)。
pub fn mark_done(
    storage: &Storage,
    message_id: &str,
    address: &str,
) -> Result<Option<StatusChange>, RoutingError> {
    let conn = storage.conn();
    let old_status: Option<String> = conn
        .query_row(
            "SELECT status FROM messages WHERE id = ?1 AND to_address = ?2 AND status != 'done'",
            [message_id, address],
            |row| row.get(0),
        )
        .optional()?;
    let old_status = match old_status {
        Some(s) => s,
        None => {
            // 确认消息存在但已经是 done，返回 None；不存在才报错
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM messages WHERE id = ?1 AND to_address = ?2",
                    [message_id, address],
                    |_| Ok(true),
                )
                .optional()?
                .unwrap_or(false);
            if exists {
                return Ok(None);
            }
            return Err(RoutingError::MessageNotFound(message_id.to_string()));
        }
    };
    conn.execute(
        "UPDATE messages SET status = 'done' WHERE id = ?1 AND to_address = ?2 AND status != 'done'",
        [message_id, address],
    )?;
    Ok(Some(super::StatusChange {
        message_id: message_id.to_string(),
        old_status,
        new_status: "done".to_string(),
    }))
}
