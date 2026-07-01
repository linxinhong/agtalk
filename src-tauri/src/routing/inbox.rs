//! inbox：一次性查询收件箱。

use super::{Message, RoutingError};
use crate::storage::Storage;
use rusqlite::params;

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
pub fn mark_read(storage: &Storage, message_id: &str) -> Result<(), RoutingError> {
    let conn = storage.conn();
    conn.execute(
        "UPDATE messages SET status = 'read' WHERE id = ?1 AND status IN ('pending', 'delivered')",
        [message_id],
    )?;
    Ok(())
}
