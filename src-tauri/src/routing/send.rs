//! send：按 UUID 投递消息，分配 event_id。

use super::{Message, RoutingError, SendRequest};
use crate::identity::mailbox;
use crate::storage::Storage;
use rusqlite::params;
use serde_json::Value;
use uuid::Uuid;

pub fn send(storage: &Storage, req: SendRequest<'_>) -> Result<Message, RoutingError> {
    if mailbox::get_by_address(storage, req.to)?.is_none() {
        return Err(RoutingError::MailboxNotFound(req.to.to_string()));
    }

    let metadata = merge_more_coming(req.metadata, req.more_coming)?;

    let id = Uuid::new_v4().to_string();
    let mut conn = storage.conn();
    let tx = conn.transaction()?;

    let event_id: i64 = tx
        .query_row(
            "UPDATE event_sequences SET last_event_id = last_event_id + 1 WHERE address = ?1 RETURNING last_event_id",
            [req.to],
            |row| row.get(0),
        )
        .map_err(|_| RoutingError::EventIdAllocation)?;

    let now = unix_timestamp();
    tx.execute(
        "INSERT INTO messages (id, to_address, to_name, from_address, from_name, body, content_type, reply_to_id, metadata, event_id, status, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'pending', ?11)",
        params![
            id,
            req.to,
            req.to_name,
            req.from,
            req.from_name,
            req.body,
            req.content_type,
            req.reply_to_id,
            metadata,
            event_id,
            now
        ],
    )?;

    tx.commit()?;

    Ok(Message {
        id,
        to_address: req.to.to_string(),
        to_name: req.to_name.to_string(),
        from_address: req.from.to_string(),
        from_name: req.from_name.to_string(),
        body: req.body.to_string(),
        content_type: req.content_type.to_string(),
        reply_to_id: req.reply_to_id.map(|s| s.to_string()),
        metadata,
        event_id,
        status: "pending".to_string(),
        created_at: now,
    })
}

fn merge_more_coming(metadata: &str, more_coming: bool) -> Result<String, RoutingError> {
    let mut meta: Value =
        serde_json::from_str(metadata).unwrap_or(Value::Object(serde_json::Map::new()));
    if let Value::Object(ref mut map) = meta {
        map.insert("more_coming".to_string(), Value::Bool(more_coming));
    }
    Ok(meta.to_string())
}

fn unix_timestamp() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
