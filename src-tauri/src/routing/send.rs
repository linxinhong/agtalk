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

    let id = Uuid::new_v4().to_string();
    let mut conn = storage.conn();
    let tx = conn.transaction()?;
    let msg = insert_message(&tx, &id, &req)?;
    tx.commit()?;
    Ok(msg)
}

/// 在调用方事务内插入消息：分配 event_id（按接收方地址）并写入 messages。
///
/// 供需要把消息插入与其它写入原子化的调用方复用（如 human 跨端事件幂等：
/// receipt 与消息插入必须在同一事务提交，消除占位 receipt 的崩溃窗口）。
pub(crate) fn insert_message(
    tx: &rusqlite::Transaction<'_>,
    id: &str,
    req: &SendRequest<'_>,
) -> Result<Message, RoutingError> {
    let metadata = merge_more_coming(req.metadata, req.more_coming)?;
    let subject = normalize_subject(req.subject);

    let event_id: i64 = tx
        .query_row(
            "UPDATE event_sequences SET last_event_id = last_event_id + 1 WHERE address = ?1 RETURNING last_event_id",
            [req.to],
            |row| row.get(0),
        )
        .map_err(|_| RoutingError::EventIdAllocation)?;

    let now = unix_timestamp();
    tx.execute(
        "INSERT INTO messages (id, to_address, to_name, from_address, from_name, body, content_type, reply_to_id, subject, metadata, event_id, status, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'pending', ?12)",
        params![
            id,
            req.to,
            req.to_name,
            req.from,
            req.from_name,
            req.body,
            req.content_type,
            req.reply_to_id,
            subject,
            metadata,
            event_id,
            now
        ],
    )?;

    Ok(Message {
        id: id.to_string(),
        to_address: req.to.to_string(),
        to_name: req.to_name.to_string(),
        from_address: req.from.to_string(),
        from_name: req.from_name.to_string(),
        body: req.body.to_string(),
        content_type: req.content_type.to_string(),
        reply_to_id: req.reply_to_id.map(|s| s.to_string()),
        subject,
        metadata,
        event_id,
        status: "pending".to_string(),
        created_at: now,
    })
}

fn normalize_subject(subject: Option<&str>) -> Option<String> {
    subject.and_then(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
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
