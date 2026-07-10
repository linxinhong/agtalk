//! reply：回复消息，支持审批 choice。

use super::{Message, RoutingError, StatusChange};
use crate::storage::Storage;
use rusqlite::params;
use rusqlite::OptionalExtension;
use serde_json::Value;
use uuid::Uuid;

/// 返回 (reply_message, original_message_status_change)。
/// original_message_status_change 仅当原消息状态真实改变时为 Some。
pub fn reply(
    storage: &Storage,
    original_id: &str,
    from: &str,
    from_name: &str,
    body: &str,
    choice: Option<&str>,
) -> Result<(Message, Option<StatusChange>), RoutingError> {
    let original: Message = {
        let conn = storage.conn();
        conn.query_row(
            "SELECT * FROM messages WHERE id = ?1",
            [original_id],
            super::Message::from_row,
        )
        .map_err(|_| RoutingError::MessageNotFound(original_id.to_string()))?
    };

    let to = original.from_address.clone();
    let to_name = original.from_name.clone();
    let content_type = if choice.is_some() {
        "approval_response"
    } else {
        "text"
    };

    let id = Uuid::new_v4().to_string();
    let mut conn = storage.conn();
    let tx = conn.transaction()?;

    let event_id: i64 = tx
        .query_row(
            "UPDATE event_sequences SET last_event_id = last_event_id + 1 WHERE address = ?1 RETURNING last_event_id",
            [&to],
            |row| row.get(0),
        )
        .map_err(|_| RoutingError::EventIdAllocation)?;

    let metadata = if let Some(c) = choice {
        let mut meta: Value = serde_json::from_str(&original.metadata)?;
        meta["response_choice"] = Value::String(c.to_string());
        meta.to_string()
    } else {
        original.metadata.clone()
    };

    let now = unix_timestamp();
    tx.execute(
        "INSERT INTO messages (id, to_address, to_name, from_address, from_name, body, content_type, reply_to_id, metadata, event_id, status, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'pending', ?11)",
        params![
            id,
            &to,
            &to_name,
            from,
            from_name,
            body,
            content_type,
            original_id,
            metadata,
            event_id,
            now
        ],
    )?;

    let original_status_change = if choice.is_some() {
        let old_status: Option<String> = tx
            .query_row(
                "SELECT status FROM messages WHERE id = ?1 AND status != 'done'",
                [original_id],
                |row| row.get(0),
            )
            .optional()?;
        tx.execute(
            "UPDATE messages SET status = 'done' WHERE id = ?1 AND status != 'done'",
            [original_id],
        )?;
        old_status.map(|s| StatusChange {
            message_id: original_id.to_string(),
            old_status: s,
            new_status: "done".to_string(),
        })
    } else {
        let old_status: Option<String> = tx
            .query_row(
                "SELECT status FROM messages WHERE id = ?1 AND status IN ('pending', 'delivered')",
                [original_id],
                |row| row.get(0),
            )
            .optional()?;
        tx.execute(
            "UPDATE messages SET status = 'read' WHERE id = ?1 AND status IN ('pending', 'delivered')",
            [original_id],
        )?;
        old_status.map(|s| StatusChange {
            message_id: original_id.to_string(),
            old_status: s,
            new_status: "read".to_string(),
        })
    };

    tx.commit()?;

    Ok((
        Message {
            id,
            to_address: to,
            to_name,
            from_address: from.to_string(),
            from_name: from_name.to_string(),
            body: body.to_string(),
            content_type: content_type.to_string(),
            reply_to_id: Some(original_id.to_string()),
            metadata,
            event_id,
            status: "pending".to_string(),
            created_at: now,
        },
        original_status_change,
    ))
}

fn unix_timestamp() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
