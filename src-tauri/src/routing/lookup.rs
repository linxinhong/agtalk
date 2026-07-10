//! lookup：按 name 查询候选 mailbox，以及消息详情。

use super::{Message, RoutingError, StatusChange};
use crate::identity::mailbox;
use crate::storage::Storage;
use rusqlite::{params, OptionalExtension};

pub fn lookup(
    storage: &Storage,
    name_filter: Option<&str>,
) -> Result<Vec<mailbox::Mailbox>, RoutingError> {
    let mbs = mailbox::list(storage, name_filter)?;
    Ok(mbs)
}

const MIN_SHORT_ID_LEN: usize = 8;

/// 将短 ID 前缀解析为完整 UUID。
/// 搜索范围限定为该 address 收到或发出的消息。
/// 0 条 → MessageNotFound；≥2 条 → MessageIdAmbiguous。
pub fn resolve_id(
    storage: &Storage,
    address: &str,
    short_id: &str,
) -> Result<String, RoutingError> {
    if short_id.len() < MIN_SHORT_ID_LEN {
        return Err(RoutingError::MessageIdTooShort(short_id.to_string()));
    }

    let conn = storage.conn();
    let mut stmt = conn.prepare(
        "SELECT id FROM messages WHERE (to_address = ?1 OR from_address = ?1) AND id LIKE ?2 || '%'",
    )?;
    let ids: Vec<String> = stmt
        .query_map(params![address, short_id], |row| row.get::<_, String>(0))?
        .collect::<Result<_, _>>()?;

    match ids.len() {
        0 => Err(RoutingError::MessageNotFound(short_id.to_string())),
        1 => Ok(ids.into_iter().next().unwrap()),
        count => Err(RoutingError::MessageIdAmbiguous {
            short_id: short_id.to_string(),
            count,
        }),
    }
}

/// 查询单条消息详情（不标记已读）。
pub fn detail(storage: &Storage, message_id: &str) -> Result<Option<Message>, RoutingError> {
    let conn = storage.conn();
    let mut stmt = conn.prepare("SELECT * FROM messages WHERE id = ?1")?;
    let msg = stmt.query_row([message_id], Message::from_row).optional()?;
    Ok(msg)
}

/// 查询消息详情并标记已读。
/// 返回消息及真实状态变化；未发生状态变化时第二项为 None。
pub fn detail_and_mark_read(
    storage: &Storage,
    address: &str,
    message_id: &str,
) -> Result<Option<(Message, Option<StatusChange>)>, RoutingError> {
    let conn = storage.conn();
    let msg: Option<Message> = if message_id == "-" {
        // 默认读取最新一条真正未读（pending/delivered）消息；没有则空。
        conn.query_row(
            "SELECT * FROM messages WHERE to_address = ?1 AND status IN ('pending', 'delivered') \
             ORDER BY event_id DESC LIMIT 1",
            [address],
            Message::from_row,
        )
        .optional()?
    } else {
        let mut stmt = conn.prepare("SELECT * FROM messages WHERE id = ?1")?;
        stmt.query_row([message_id], Message::from_row).optional()?
    };

    if let Some(ref m) = msg {
        let old_status: Option<String> = conn
            .query_row(
                "SELECT status FROM messages WHERE id = ?1 AND status IN ('pending', 'delivered')",
                [&m.id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(old_status) = old_status {
            conn.execute(
                "UPDATE messages SET status = 'read' WHERE id = ?1 AND status IN ('pending', 'delivered')",
                [&m.id],
            )?;
            return Ok(Some((
                m.clone(),
                Some(StatusChange {
                    message_id: m.id.clone(),
                    old_status,
                    new_status: "read".to_string(),
                }),
            )));
        }
        return Ok(Some((m.clone(), None)));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;

    fn insert_message(storage: &Storage, id: &str, to: &str, from: &str, event_id: i64) {
        // 用 revive 确保以 address 为键的 mailbox + event_sequences 存在。
        crate::identity::mailbox::revive(storage, to, "recv", "", "").unwrap();
        crate::identity::mailbox::revive(storage, from, "send", "", "").unwrap();
        let conn = storage.conn();
        conn.execute(
            "INSERT INTO messages (id, to_address, to_name, from_address, from_name, body, content_type, reply_to_id, metadata, event_id, status, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'pending', ?11)",
            params![id, to, "recv", from, "send", "body", "text", Option::<String>::None, "{}", event_id, 1.0],
        )
        .unwrap();
    }

    #[test]
    fn resolve_id_exact_uuid() {
        let storage = Storage::open_in_memory().unwrap();
        let id = "6f0d4353-f4b2-46ab-afb6-8c7f6af02049";
        insert_message(&storage, id, "addr-a", "addr-b", 1);

        let resolved = resolve_id(&storage, "addr-a", id).unwrap();
        assert_eq!(resolved, id);
    }

    #[test]
    fn resolve_id_short_prefix() {
        let storage = Storage::open_in_memory().unwrap();
        let id = "6f0d4353-f4b2-46ab-afb6-8c7f6af02049";
        insert_message(&storage, id, "addr-a", "addr-b", 1);

        let resolved = resolve_id(&storage, "addr-a", "6f0d4353").unwrap();
        assert_eq!(resolved, id);
    }

    #[test]
    fn resolve_id_too_short() {
        let storage = Storage::open_in_memory().unwrap();
        let err = resolve_id(&storage, "addr-a", "6f0d435").unwrap_err();
        assert!(matches!(err, RoutingError::MessageIdTooShort(_)));
    }

    #[test]
    fn resolve_id_not_found() {
        let storage = Storage::open_in_memory().unwrap();
        let err = resolve_id(&storage, "addr-a", "6f0d4353").unwrap_err();
        assert!(matches!(err, RoutingError::MessageNotFound(_)));
    }

    #[test]
    fn resolve_id_ambiguous() {
        let storage = Storage::open_in_memory().unwrap();
        insert_message(
            &storage,
            "6f0d4353-f4b2-46ab-afb6-8c7f6af02049",
            "addr-a",
            "addr-b",
            1,
        );
        insert_message(
            &storage,
            "6f0d4353-aaaa-46ab-afb6-8c7f6af02049",
            "addr-a",
            "addr-b",
            2,
        );

        let err = resolve_id(&storage, "addr-a", "6f0d4353").unwrap_err();
        assert!(matches!(err, RoutingError::MessageIdAmbiguous { .. }));
    }
}
