//! SQLite 存储层：schema migration 和数据库操作。

use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use std::path::PathBuf;
use std::sync::Mutex;

const CURRENT_VERSION: u32 = 3;

pub struct Storage {
    conn: Mutex<Connection>,
}

fn db_path() -> PathBuf { crate::paths::db_path() }

fn ensure_column(conn: &Connection, table: &str, column: &str, column_def: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for col in columns {
        if col? == column {
            return Ok(());
        }
    }
    conn.execute_batch(&format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, column_def))?;
    Ok(())
}

impl Storage {
    pub fn open() -> Result<Self> {
        let path = db_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("无法创建目录: {:?}", parent))?;
        }
        let conn = Connection::open(&path)
            .with_context(|| format!("无法打开数据库: {:?}", path))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let storage = Self { conn: Mutex::new(conn) };
        storage.migrate()?;
        Ok(storage)
    }

    #[allow(dead_code)]
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let storage = Self { conn: Mutex::new(conn) };
        storage.migrate()?;
        Ok(storage)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let version: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(SCHEMA_V1)?;
        }
        ensure_column(&conn, "participants", "capabilities", "TEXT NOT NULL DEFAULT ''")?;
        ensure_column(&conn, "conversations", "kind", "TEXT NOT NULL DEFAULT 'direct'")?;
        ensure_column(&conn, "messages", "correlation_id", "TEXT")?;
        ensure_column(&conn, "messages", "metadata", "TEXT NOT NULL DEFAULT '{}'")?;
        if version < CURRENT_VERSION {
            conn.pragma_update(None, "user_version", CURRENT_VERSION)?;
        }
        Ok(())
    }

    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap()
    }
}

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS participants (
    id              TEXT PRIMARY KEY,
    name            TEXT UNIQUE NOT NULL,
    type            TEXT NOT NULL DEFAULT 'agent',
    display_name    TEXT NOT NULL DEFAULT '',
    transport       TEXT NOT NULL DEFAULT 'terminal',
    transport_config TEXT NOT NULL DEFAULT '{}',
    capabilities    TEXT NOT NULL DEFAULT '',
    status          TEXT NOT NULL DEFAULT 'offline',
    last_seen_at    REAL NOT NULL DEFAULT (unixepoch('subsec')),
    created_at      REAL NOT NULL DEFAULT (unixepoch('subsec'))
);

CREATE TABLE IF NOT EXISTS conversations (
    id              TEXT PRIMARY KEY,
    title           TEXT NOT NULL DEFAULT '',
    kind            TEXT NOT NULL DEFAULT 'direct',
    created_at      REAL NOT NULL DEFAULT (unixepoch('subsec')),
    updated_at      REAL NOT NULL DEFAULT (unixepoch('subsec'))
);

CREATE TABLE IF NOT EXISTS conversation_participants (
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    participant_id  TEXT NOT NULL REFERENCES participants(id),
    joined_at       REAL NOT NULL DEFAULT (unixepoch('subsec')),
    PRIMARY KEY (conversation_id, participant_id)
);

CREATE TABLE IF NOT EXISTS messages (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    sender_id       TEXT NOT NULL REFERENCES participants(id),
    body            TEXT NOT NULL,
    content_type    TEXT NOT NULL DEFAULT 'text',
    correlation_id  TEXT,
    status          TEXT NOT NULL DEFAULT 'pending',
    reply_to_id     TEXT,
    metadata        TEXT NOT NULL DEFAULT '{}',
    created_at      REAL NOT NULL DEFAULT (unixepoch('subsec'))
);

CREATE TABLE IF NOT EXISTS message_recipients (
    message_id      TEXT NOT NULL REFERENCES messages(id),
    recipient_id    TEXT NOT NULL REFERENCES participants(id),
    status          TEXT NOT NULL DEFAULT 'pending',
    delivered_at    REAL,
    read_at         REAL,
    PRIMARY KEY (message_id, recipient_id)
);

CREATE INDEX IF NOT EXISTS idx_messages_conv ON messages(conversation_id, created_at);
CREATE INDEX IF NOT EXISTS idx_messages_reply ON messages(reply_to_id);
CREATE INDEX IF NOT EXISTS idx_msg_recipients_rcpt ON message_recipients(recipient_id, status);
CREATE INDEX IF NOT EXISTS idx_conv_parts_participant ON conversation_participants(participant_id);
"#;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Participant {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub participant_type: String,
    pub display_name: String,
    pub transport: String,
    pub transport_config: String,
    pub capabilities: String,
    pub status: String,
    pub last_seen_at: f64,
    pub created_at: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub participants: Vec<String>,
    pub last_message: Option<MessagePreview>,
    pub unread_count: u32,
    pub created_at: f64,
    pub updated_at: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MessagePreview {
    pub id: String,
    pub sender_name: String,
    pub body: String,
    pub created_at: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub body: String,
    pub content_type: String,
    pub status: String,
    pub correlation_id: Option<String>,
    pub reply_to_id: Option<String>,
    pub metadata: String,
    pub recipients: Vec<RecipientStatus>,
    pub created_at: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RecipientStatus {
    pub recipient_id: String,
    pub recipient_name: String,
    pub status: String,
    pub delivered_at: Option<f64>,
    pub read_at: Option<f64>,
}

// ─── 参与者 CRUD ──────────────────────────────────────

impl Storage {
    pub fn register_participant(
        &self,
        name: &str,
        participant_type: &str,
        display_name: &str,
        transport: &str,
        transport_config: &str,
    ) -> Result<Participant> {
        let id = uuid::Uuid::new_v4().to_string();
        let conn = self.conn();
        conn.execute(
            "INSERT INTO participants (id, name, type, display_name, transport, transport_config, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'online')",
            params![id, name, participant_type, display_name, transport, transport_config],
        )?;
        Ok(get_participant_row(&conn, &id)?)
    }

    pub fn unregister_participant(&self, name: &str) -> Result<()> {
        self.conn().execute("DELETE FROM participants WHERE name = ?1", params![name])?;
        Ok(())
    }

    pub fn get_participant_by_name(&self, name: &str) -> Result<Option<Participant>> {
        get_participant_by_name_impl(&self.conn(), name)
    }

    pub fn list_participants(&self, participant_type: Option<&str>) -> Result<Vec<Participant>> {
        let conn = self.conn();
        let sql = match participant_type {
            Some(_) => "SELECT * FROM participants WHERE type = ?1 ORDER BY name",
            None => "SELECT * FROM participants ORDER BY name",
        };
        let mut stmt = conn.prepare(sql)?;
        let rows: Vec<Participant> = match participant_type {
            Some(t) => stmt.query_map(params![t], |row| row_to_participant(row))?.filter_map(|r| r.ok()).collect(),
            None => stmt.query_map([], |row| row_to_participant(row))?.filter_map(|r| r.ok()).collect(),
        };
        Ok(rows)
    }

    #[allow(dead_code)]
    pub fn update_participant_status(&self, name: &str, status: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE participants SET status = ?1, last_seen_at = unixepoch('subsec') WHERE name = ?2",
            params![status, name],
        )?;
        Ok(())
    }
}

// ─── 消息操作 ────────────────────────────────────────

impl Storage {
    pub fn send_message(
        &self,
        sender_name: &str,
        to_names: &[String],
        body: &str,
        content_type: &str,
        reply_to: Option<&str>,
        conversation_id: Option<&str>,
        correlation_id: Option<&str>,
        conversation_kind: Option<&str>,
        metadata: Option<&str>,
    ) -> Result<Message> {
        let conn = self.conn();
        let sender = get_participant_row(&conn, sender_name)?;
        let sender_id = &sender.id;

        let conv_id = match conversation_id {
            Some(cid) => cid.to_string(),
            None => self.find_or_create_conversation(&conn, sender_id, to_names, conversation_kind)?,
            
        };

        let msg_id = uuid::Uuid::new_v4().to_string();

        let metadata_val = metadata.unwrap_or("{}");
        conn.execute(
            "INSERT INTO messages (id, conversation_id, sender_id, body, content_type, reply_to_id, correlation_id, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![msg_id, conv_id, sender_id, body, content_type, reply_to, correlation_id, metadata_val],
        )?;

        for to_name in to_names {
            if let Ok(recipient) = get_participant_row(&conn, to_name) {
                conn.execute(
                    "INSERT INTO message_recipients (message_id, recipient_id) VALUES (?1, ?2)",
                    params![msg_id, recipient.id],
                )?;
            }
        }

        conn.execute(
            "UPDATE conversations SET updated_at = unixepoch('subsec') WHERE id = ?1",
            params![conv_id],
        )?;

        let recipients = self.get_recipients_for_msg(&conn, &msg_id)?;
        let created_at: f64 = conn.query_row(
            "SELECT created_at FROM messages WHERE id = ?1",
            params![msg_id],
            |r| r.get(0),
        )?;
        Ok(Message {
            id: msg_id,
            conversation_id: conv_id,
            sender_id: sender.id.clone(),
            sender_name: sender.name.clone(),
            body: body.to_string(),
            content_type: content_type.to_string(),
            metadata: metadata_val.to_string(),
            correlation_id: correlation_id.map(|s| s.to_string()),
            status: "pending".to_string(),
            reply_to_id: reply_to.map(|s| s.to_string()),
            recipients,
            created_at,
        })
    }

    pub fn get_messages(
        &self,
        conversation_id: &str,
        limit: u32,
        before_id: Option<&str>,
    ) -> Result<Vec<Message>> {
        let conn = self.conn();
        let sql = match before_id {
            Some(_) => concat!(
                "SELECT m.*, p.name as sname FROM messages m ",
                "JOIN participants p ON m.sender_id = p.id ",
                "WHERE m.conversation_id = ?1 AND m.created_at < ",
                "(SELECT created_at FROM messages WHERE id = ?2) ",
                "ORDER BY m.created_at DESC LIMIT ?3"
            ),
            None => concat!(
                "SELECT m.*, p.name as sname FROM messages m ",
                "JOIN participants p ON m.sender_id = p.id ",
                "WHERE m.conversation_id = ?1 ",
                "ORDER BY m.created_at DESC LIMIT ?2"
            ),
        };

        let mut stmt = conn.prepare(sql)?;
        let rows: Vec<Message> = match before_id {
            Some(before) => stmt.query_map(
                params![conversation_id, before, limit],
                |row| row_to_message(row),
            )?.filter_map(|r| r.ok()).collect(),
            None => stmt.query_map(
                params![conversation_id, limit],
                |row| row_to_message(row),
            )?.filter_map(|r| r.ok()).collect(),
        };

        let mut messages = Vec::new();
        for mut msg in rows {
            msg.recipients = self.get_recipients_for_msg(&conn, &msg.id)?;
            messages.push(msg);
        }
        messages.reverse();
        Ok(messages)
    }

    pub fn list_conversations(&self, participant_name: Option<&str>) -> Result<Vec<Conversation>> {
        let conn = self.conn();
        let sql = match participant_name {
            Some(_) => concat!(
                "SELECT DISTINCT c.id, c.title, c.kind, c.created_at, c.updated_at FROM conversations c ",
                "JOIN conversation_participants cp ON c.id = cp.conversation_id ",
                "JOIN participants p ON cp.participant_id = p.id ",
                "WHERE p.name = ?1 OR p.id = ?1 ",
                "ORDER BY c.updated_at DESC"
            ),
            None => "SELECT id, title, kind, created_at, updated_at FROM conversations ORDER BY updated_at DESC",
        };

        let mut stmt = conn.prepare(sql)?;
        let rows: Vec<(String, String, String, f64, f64)> = match participant_name {
            Some(name) => stmt.query_map(params![name], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
            })?.filter_map(|r| r.ok()).collect(),
            None => stmt.query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
            })?.filter_map(|r| r.ok()).collect(),
        };

        let mut conversations = Vec::new();
        for (id, title, kind, created_at, updated_at) in rows {
            let participants = self.get_conv_participant_names(&conn, &id)?;
            let (last_message, unread_count) =
                self.get_conversation_summary(&conn, &id, participant_name)?;
            conversations.push(Conversation {
                id, title, kind, participants, last_message, unread_count, created_at, updated_at,
            });
        }
        Ok(conversations)
    }

    pub fn mark_done(&self, msg_id: &str, participant_name: &str) -> Result<()> {
        let conn = self.conn();
        let p = get_participant_row(&conn, participant_name)?;
        conn.execute(
            "UPDATE message_recipients SET status = 'done' WHERE message_id = ?1 AND recipient_id = ?2",
            params![msg_id, p.id],
        )?;
        Ok(())
    }

    pub fn mark_read(&self, msg_id: &str, participant_name: &str) -> Result<()> {
        let conn = self.conn();
        let p = get_participant_row(&conn, participant_name)?;
        conn.execute(
            "UPDATE message_recipients SET status = 'read', read_at = unixepoch('subsec')
             WHERE message_id = ?1 AND recipient_id = ?2",
            params![msg_id, p.id],
        )?;
        Ok(())
    }

    pub fn mark_delivered(&self, msg_id: &str, participant_name: &str) -> Result<()> {
        let conn = self.conn();
        let p = get_participant_row(&conn, participant_name)?;
        conn.execute(
            "UPDATE message_recipients SET status = 'delivered', delivered_at = unixepoch('subsec')
             WHERE message_id = ?1 AND recipient_id = ?2",
            params![msg_id, p.id],
        )?;
        Ok(())
    }
}

// ─── 内部辅助 ────────────────────────────────────────

impl Storage {
    fn find_or_create_conversation(
        &self,
        conn: &Connection,
        sender_id: &str,
        to_names: &[String],
        kind: Option<&str>,
    ) -> Result<String> {
        if to_names.len() == 1 {
            if let Ok(other) = get_participant_row(conn, &to_names[0]) {
                if let Ok(Some(cid)) = conn.query_row(
                    "SELECT cp1.conversation_id FROM conversation_participants cp1
                     JOIN conversation_participants cp2 ON cp1.conversation_id = cp2.conversation_id
                     WHERE cp1.participant_id = ?1 AND cp2.participant_id = ?2
                     AND (SELECT COUNT(*) FROM conversation_participants WHERE conversation_id = cp1.conversation_id) = 2
                     LIMIT 1",
                    params![sender_id, other.id],
                    |row| row.get(0),
                ) {
                    return Ok(cid);
                }
            }
        }

        let conv_id = uuid::Uuid::new_v4().to_string();
        let title = to_names.join(", ");
        let conv_kind = kind.unwrap_or("direct");
        conn.execute(
            "INSERT INTO conversations (id, title, kind) VALUES (?1, ?2, ?3)",
            params![conv_id, title, conv_kind],
        )?;

        let mut ids = vec![sender_id.to_string()];
        for name in to_names {
            if let Ok(p) = get_participant_row(conn, name) {
                if !ids.contains(&p.id) {
                    ids.push(p.id);
                }
            }
        }
        for pid in &ids {
            conn.execute(
                "INSERT OR IGNORE INTO conversation_participants (conversation_id, participant_id) VALUES (?1, ?2)",
                params![conv_id, pid],
            )?;
        }
        Ok(conv_id)
    }

    fn get_recipients_for_msg(&self, conn: &Connection, msg_id: &str) -> Result<Vec<RecipientStatus>> {
        let mut stmt = conn.prepare(
            "SELECT mr.recipient_id, p.name, mr.status, mr.delivered_at, mr.read_at
             FROM message_recipients mr JOIN participants p ON mr.recipient_id = p.id
             WHERE mr.message_id = ?1",
        )?;
        let rows = stmt.query_map(params![msg_id], |row| {
            Ok(RecipientStatus {
                recipient_id: row.get(0)?,
                recipient_name: row.get(1)?,
                status: row.get(2)?,
                delivered_at: row.get(3)?,
                read_at: row.get(4)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    fn get_conv_participant_names(&self, conn: &Connection, conv_id: &str) -> Result<Vec<String>> {
        let mut stmt = conn.prepare(
            "SELECT p.name FROM conversation_participants cp
             JOIN participants p ON cp.participant_id = p.id
             WHERE cp.conversation_id = ?1",
        )?;
        let rows = stmt.query_map(params![conv_id], |row| row.get(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    fn get_conversation_summary(
        &self,
        conn: &Connection,
        conv_id: &str,
        viewer: Option<&str>,
    ) -> Result<(Option<MessagePreview>, u32)> {
        let last = conn.query_row(
            "SELECT m.id, p.name, m.body, m.created_at
             FROM messages m JOIN participants p ON m.sender_id = p.id
             WHERE m.conversation_id = ?1 ORDER BY m.created_at DESC LIMIT 1",
            params![conv_id],
            |row| Ok(MessagePreview {
                id: row.get(0)?,
                sender_name: row.get(1)?,
                body: row.get(2)?,
                created_at: row.get(3)?,
            }),
        ).ok();

        let unread = match viewer {
            Some(name) => conn.query_row(
                "SELECT COUNT(*) FROM message_recipients mr
                 JOIN messages m ON mr.message_id = m.id
                 JOIN participants p ON mr.recipient_id = p.id
                 WHERE m.conversation_id = ?1 AND p.name = ?2 AND mr.status IN ('pending','delivered')",
                params![conv_id, name],
                |row| row.get(0),
            ).unwrap_or(0),
            None => 0,
        };

        Ok((last, unread))
    }
}

fn get_participant_row(conn: &Connection, name_or_id: &str) -> Result<Participant> {
    conn.query_row(
        "SELECT * FROM participants WHERE name = ?1 OR id = ?1",
        params![name_or_id],
        |row| row_to_participant(row),
    ).map_err(|_| anyhow::anyhow!("参与者不存在: {}", name_or_id))
}

fn get_participant_by_name_impl(conn: &Connection, name: &str) -> Result<Option<Participant>> {
    match conn.query_row(
        "SELECT * FROM participants WHERE name = ?1",
        params![name],
        |row| row_to_participant(row),
    ) {
        Ok(p) => Ok(Some(p)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn row_to_participant(row: &rusqlite::Row) -> rusqlite::Result<Participant> {
    Ok(Participant {
        id: row.get("id")?,
        name: row.get("name")?,
        participant_type: row.get("type")?,
        display_name: row.get("display_name")?,
        transport: row.get("transport")?,
        transport_config: row.get("transport_config")?,
        capabilities: row.get("capabilities")?,
        status: row.get("status")?,
        last_seen_at: row.get("last_seen_at")?,
        created_at: row.get("created_at")?,
    })
}

fn row_to_message(row: &rusqlite::Row) -> rusqlite::Result<Message> {
    Ok(Message {
        id: row.get("id")?,
        conversation_id: row.get("conversation_id")?,
        sender_id: row.get("sender_id")?,
        sender_name: row.get("sname")?,
        body: row.get("body")?,
        content_type: row.get("content_type")?,
        correlation_id: row.get("correlation_id")?,
        metadata: row.get::<_, String>("metadata").unwrap_or_default(),
        status: row.get("status")?,
        reply_to_id: row.get("reply_to_id")?,
        recipients: Vec::new(),
        created_at: row.get("created_at")?,
    })
}

#[cfg(test)]
mod migration_tests {
    use super::*;

    #[test]
    fn migrates_old_schema_without_column_order_breakage() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            PRAGMA foreign_keys=ON;
            CREATE TABLE participants (
                id TEXT PRIMARY KEY,
                name TEXT UNIQUE NOT NULL,
                type TEXT NOT NULL DEFAULT 'agent',
                display_name TEXT NOT NULL DEFAULT '',
                transport TEXT NOT NULL DEFAULT 'terminal',
                transport_config TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'offline',
                last_seen_at REAL NOT NULL DEFAULT 1.0,
                created_at REAL NOT NULL DEFAULT 1.0
            );
            CREATE TABLE conversations (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL DEFAULT '',
                created_at REAL NOT NULL DEFAULT 1.0,
                updated_at REAL NOT NULL DEFAULT 1.0
            );
            CREATE TABLE conversation_participants (
                conversation_id TEXT NOT NULL,
                participant_id TEXT NOT NULL,
                joined_at REAL NOT NULL DEFAULT 1.0,
                PRIMARY KEY (conversation_id, participant_id)
            );
            CREATE TABLE messages (
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                sender_id TEXT NOT NULL,
                body TEXT NOT NULL,
                content_type TEXT NOT NULL DEFAULT 'text',
                status TEXT NOT NULL DEFAULT 'pending',
                reply_to_id TEXT,
                created_at REAL NOT NULL DEFAULT 2.0
            );
            CREATE TABLE message_recipients (
                message_id TEXT NOT NULL,
                recipient_id TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                delivered_at REAL,
                read_at REAL,
                PRIMARY KEY (message_id, recipient_id)
            );
            INSERT INTO participants (id, name, type, display_name, transport, transport_config, status, last_seen_at, created_at)
            VALUES ('p1', 'alice', 'agent', 'Alice', 'terminal', '{}', 'online', 10.0, 11.0),
                   ('p2', 'bob', 'agent', 'Bob', 'terminal', '{}', 'online', 12.0, 13.0);
            INSERT INTO conversations (id, title, created_at, updated_at)
            VALUES ('c1', 'old chat', 20.0, 21.0);
            INSERT INTO conversation_participants (conversation_id, participant_id)
            VALUES ('c1', 'p1'), ('c1', 'p2');
            INSERT INTO messages (id, conversation_id, sender_id, body, content_type, status, reply_to_id, created_at)
            VALUES ('m1', 'c1', 'p1', 'hello', 'text', 'pending', NULL, 30.0);
            INSERT INTO message_recipients (message_id, recipient_id)
            VALUES ('m1', 'p2');
            PRAGMA user_version = 1;
            "#,
        ).unwrap();

        let storage = Storage { conn: Mutex::new(conn) };
        storage.migrate().unwrap();

        let alice = storage.get_participant_by_name("alice").unwrap().unwrap();
        assert_eq!(alice.status, "online");
        assert_eq!(alice.capabilities, "");

        let conversations = storage.list_conversations(Some("bob")).unwrap();
        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].kind, "direct");
        assert_eq!(conversations[0].created_at, 20.0);

        let messages = storage.get_messages("c1", 50, None).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].sender_name, "alice");
        assert_eq!(messages[0].body, "hello");
        assert_eq!(messages[0].metadata, "{}");
        assert_eq!(messages[0].created_at, 30.0);
    }
}
