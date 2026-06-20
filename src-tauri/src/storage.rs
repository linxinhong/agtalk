//! SQLite 存储层：schema migration 和数据库操作。

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::ser::{Serialize, SerializeStruct, Serializer};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::config::AgConfig;

const CURRENT_VERSION: u32 = 9;

pub struct Storage {
    conn: Mutex<Connection>,
    config: Arc<AgConfig>,
}

fn db_path() -> PathBuf {
    crate::paths::db_path()
}

fn ensure_column(conn: &Connection, table: &str, column: &str, column_def: &str) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({})", table))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for col in columns {
        if col? == column {
            return Ok(());
        }
    }
    conn.execute_batch(&format!(
        "ALTER TABLE {} ADD COLUMN {} {}",
        table, column, column_def
    ))?;
    Ok(())
}

/// 将 f64 时间戳序列化为 ISO8601 字符串
fn serialize_iso<S: serde::Serializer>(ts: &f64, s: S) -> Result<S::Ok, S::Error> {
    let secs = ts.trunc() as i64;
    let nanos = ((ts - ts.trunc()) * 1_000_000_000.0) as u32;
    let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nanos).unwrap_or_default();
    s.serialize_str(&dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

/// 将 Option<f64> 时间戳序列化为 Option<ISO8601 字符串>
fn serialize_iso_opt<S: serde::Serializer>(ts: &Option<f64>, s: S) -> Result<S::Ok, S::Error> {
    match ts {
        Some(ts) => {
            let secs = ts.trunc() as i64;
            let nanos = ((ts - ts.trunc()) * 1_000_000_000.0) as u32;
            let dt =
                chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nanos).unwrap_or_default();
            s.serialize_some(&dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        }
        None => s.serialize_none(),
    }
}

/// 从 metadata JSON 中提取 subject 字段
fn subject_from_metadata(metadata: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(metadata)
        .ok()
        .and_then(|v| v.get("subject").and_then(|s| s.as_str().map(String::from)))
}

impl Storage {
    #[allow(dead_code)]
    pub fn open() -> Result<Self> {
        Self::open_with_config(Arc::new(AgConfig::load().unwrap_or_default()))
    }

    pub fn open_with_config(config: Arc<AgConfig>) -> Result<Self> {
        let path = db_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("无法创建目录: {:?}", parent))?;
        }
        let conn =
            Connection::open(&path).with_context(|| format!("无法打开数据库: {:?}", path))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let storage = Self {
            conn: Mutex::new(conn),
            config,
        };
        storage.migrate()?;
        Ok(storage)
    }

    #[allow(dead_code)]
    pub fn open_memory() -> Result<Self> {
        Self::open_memory_with_config(Arc::new(AgConfig::default()))
    }

    pub fn open_memory_with_config(config: Arc<AgConfig>) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        let storage = Self {
            conn: Mutex::new(conn),
            config,
        };
        storage.migrate()?;
        Ok(storage)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let version: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(SCHEMA_V1)?;
        }
        ensure_column(
            &conn,
            "participants",
            "capabilities",
            "TEXT NOT NULL DEFAULT ''",
        )?;
        ensure_column(
            &conn,
            "conversations",
            "kind",
            "TEXT NOT NULL DEFAULT 'direct'",
        )?;
        ensure_column(&conn, "messages", "correlation_id", "TEXT")?;
        ensure_column(&conn, "messages", "metadata", "TEXT NOT NULL DEFAULT '{}'")?;
        // v4: workspace + session 维度（先确保列存在，再创建索引）
        ensure_column(&conn, "participants", "workspace_id", "TEXT")?;
        ensure_column(&conn, "conversations", "workspace_id", "TEXT")?;
        ensure_column(&conn, "messages", "workspace_id", "TEXT")?;
        // v5: done_at for message_recipients
        ensure_column(&conn, "message_recipients", "done_at", "REAL")?;
        // v6: delivery session tracking + participant intro + attachments
        ensure_column(&conn, "message_recipients", "read_by_session_id", "TEXT")?;
        ensure_column(&conn, "message_recipients", "done_by_session_id", "TEXT")?;
        ensure_column(&conn, "participants", "intro", "TEXT NOT NULL DEFAULT ''")?;
        conn.execute_batch(SCHEMA_V5_ADDITIONS)?;
        conn.execute_batch(SCHEMA_V4_ADDITIONS)?;
        // v7: session 级别 notify_config（agent_sessions 在 V4 中创建）
        ensure_column(&conn, "agent_sessions", "notify_config", "TEXT")?;
        conn.execute_batch(SCHEMA_V6_ADDITIONS)?;
        // v8: peers 调度视图所需字段
        ensure_column(&conn, "participants", "role", "TEXT NOT NULL DEFAULT 'agent'")?;
        ensure_column(&conn, "agent_sessions", "runtime_config", "TEXT")?;
        // v9: session takeover 所需的 endpoint 标识
        ensure_column(&conn, "agent_sessions", "endpoint_key", "TEXT")?;
        conn.execute_batch(SCHEMA_V9_ADDITIONS)?;
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
    intro           TEXT NOT NULL DEFAULT '',
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
    done_at         REAL,
    read_by_session_id TEXT,
    done_by_session_id TEXT,
    PRIMARY KEY (message_id, recipient_id)
);

CREATE INDEX IF NOT EXISTS idx_messages_conv ON messages(conversation_id, created_at);
CREATE INDEX IF NOT EXISTS idx_messages_reply ON messages(reply_to_id);
CREATE INDEX IF NOT EXISTS idx_msg_recipients_rcpt ON message_recipients(recipient_id, status);
CREATE INDEX IF NOT EXISTS idx_conv_parts_participant ON conversation_participants(participant_id);
"#;

const SCHEMA_V5_ADDITIONS: &str = r#"
CREATE INDEX IF NOT EXISTS idx_msg_recipients_done ON message_recipients(done_at);
"#;

const SCHEMA_V4_ADDITIONS: &str = r#"
CREATE TABLE IF NOT EXISTS workspaces (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    root         TEXT NOT NULL,
    detected_by  TEXT NOT NULL DEFAULT 'cwd-scan',
    created_at   REAL NOT NULL DEFAULT (unixepoch('subsec')),
    UNIQUE(root)
);

CREATE TABLE IF NOT EXISTS agent_sessions (
    id             TEXT PRIMARY KEY,
    workspace_id   TEXT NOT NULL REFERENCES workspaces(id),
    participant_id TEXT NOT NULL REFERENCES participants(id),
    token          TEXT NOT NULL,
    status         TEXT NOT NULL DEFAULT 'active',
    notify_config  TEXT,
    created_at     REAL NOT NULL DEFAULT (unixepoch('subsec')),
    expires_at     REAL,
    last_seen_at   REAL
);

CREATE INDEX IF NOT EXISTS idx_sessions_workspace ON agent_sessions(workspace_id);
CREATE INDEX IF NOT EXISTS idx_sessions_participant ON agent_sessions(participant_id);
CREATE INDEX IF NOT EXISTS idx_participants_workspace ON participants(workspace_id);
CREATE INDEX IF NOT EXISTS idx_conversations_workspace ON conversations(workspace_id);
CREATE INDEX IF NOT EXISTS idx_messages_workspace ON messages(workspace_id);
"#;

const SCHEMA_V6_ADDITIONS: &str = r#"
CREATE TABLE IF NOT EXISTS attachments (
    id              TEXT PRIMARY KEY,
    message_id      TEXT NOT NULL REFERENCES messages(id),
    role            TEXT NOT NULL DEFAULT 'attachment',
    filename        TEXT,
    content_type    TEXT NOT NULL DEFAULT 'text/markdown',
    size            INTEGER NOT NULL DEFAULT 0,
    storage_path    TEXT NOT NULL,
    created_at      REAL NOT NULL DEFAULT (unixepoch('subsec'))
);

CREATE INDEX IF NOT EXISTS idx_attachments_message ON attachments(message_id);
"#;

const SCHEMA_V9_ADDITIONS: &str = r#"
CREATE INDEX IF NOT EXISTS idx_sessions_endpoint ON agent_sessions(workspace_id, endpoint_key, status);
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
    pub intro: String,
    pub role: String,
    pub status: String,
    pub last_seen_at: f64,
    pub created_at: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MessageCounts {
    pub unread: u32,
    pub pending: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub peers: Vec<String>,
    pub last_message: Option<MessagePreview>,
    pub counts: MessageCounts,
    #[serde(serialize_with = "serialize_iso")]
    pub created_at: f64,
    #[serde(serialize_with = "serialize_iso")]
    pub updated_at: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MessagePreview {
    pub id: String,
    pub sender_name: String,
    pub body: String,
    #[serde(serialize_with = "serialize_iso")]
    pub created_at: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Attachment {
    pub id: String,
    pub message_id: String,
    pub role: String,
    pub filename: String,
    pub content_type: String,
    pub size: usize,
    /// 内部附件为相对文件名（存于 attachment_dir），外部附件为原始绝对路径。
    #[serde(skip)]
    pub storage_path: String,
    #[serde(serialize_with = "serialize_iso")]
    pub created_at: f64,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub sender_id: String,
    pub sender_name: String,
    pub body: String,
    pub body_size: usize,
    pub content_type: String,
    pub status: String,
    pub correlation_id: Option<String>,
    pub reply_to_id: Option<String>,
    pub metadata: String,
    pub recipients: Vec<RecipientStatus>,
    pub attachments: Vec<Attachment>,
    pub full_body: Option<String>,
    pub created_at: f64,
}

impl Serialize for Message {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let subject = subject_from_metadata(&self.metadata);
        let mut m = s.serialize_struct("Message", 16)?;
        m.serialize_field("id", &self.id)?;
        m.serialize_field("chat_id", &self.conversation_id)?;
        m.serialize_field("subject", &subject)?;
        m.serialize_field("sender_id", &self.sender_id)?;
        m.serialize_field("sender_name", &self.sender_name)?;
        m.serialize_field("body", &self.body)?;
        m.serialize_field("body_size", &self.body_size)?;
        m.serialize_field("content_type", &self.content_type)?;
        m.serialize_field("status", &self.status)?;
        m.serialize_field("correlation_id", &self.correlation_id)?;
        m.serialize_field("reply_to_id", &self.reply_to_id)?;
        m.serialize_field("metadata", &self.metadata)?;
        m.serialize_field("recipients", &self.recipients)?;
        m.serialize_field("attachments", &self.attachments)?;
        m.serialize_field("full_body", &self.full_body)?;
        // created_at as ISO8601
        let secs = self.created_at.trunc() as i64;
        let nanos = ((self.created_at - self.created_at.trunc()) * 1_000_000_000.0) as u32;
        let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nanos).unwrap_or_default();
        m.serialize_field(
            "created_at",
            &dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        )?;
        m.end()
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RecipientStatus {
    pub recipient_id: String,
    pub recipient_name: String,
    pub status: String,
    #[serde(serialize_with = "serialize_iso_opt")]
    pub delivered_at: Option<f64>,
    #[serde(serialize_with = "serialize_iso_opt")]
    pub read_at: Option<f64>,
    #[serde(serialize_with = "serialize_iso_opt")]
    pub done_at: Option<f64>,
    pub read_by_session_id: Option<String>,
    pub done_by_session_id: Option<String>,
}

// ─── Inbox 待办中心 ─────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct InboxSender {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub intro: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InboxMessageContent {
    pub mode: String,
    pub body: String,
    pub truncated: bool,
    pub size: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InboxAttachment {
    pub id: String,
    pub role: String,
    pub filename: String,
    pub content_type: String,
    pub size: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InboxDelivery {
    pub status: String,
    #[serde(serialize_with = "serialize_iso_opt")]
    pub delivered_at: Option<f64>,
    #[serde(serialize_with = "serialize_iso_opt")]
    pub read_at: Option<f64>,
    #[serde(serialize_with = "serialize_iso_opt")]
    pub done_at: Option<f64>,
    pub read_by_session_id: Option<String>,
    pub done_by_session_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InboxItem {
    pub id: String,
    pub from: InboxSender,
    pub subject: Option<String>,
    pub content: InboxMessageContent,
    pub attachments: Vec<InboxAttachment>,
    pub delivery: InboxDelivery,
    pub actions: Vec<String>,
    pub action_required: bool,
    pub priority: String,
    pub kind: String,
}

// ─── 参与者 CRUD ──────────────────────────────────────

impl Storage {
    #[allow(clippy::too_many_arguments)]
    pub fn register_participant(
        &self,
        id: Option<&str>,
        name: &str,
        participant_type: &str,
        display_name: &str,
        transport: &str,
        transport_config: &str,
        intro: &str,
        role: &str,
    ) -> Result<Participant> {
        const RESERVED_NAMES: &[&str] = &["me", "human"];
        if RESERVED_NAMES.contains(&name.to_ascii_lowercase().as_str()) {
            anyhow::bail!("'{}' 是保留名称，不能注册为 participant", name);
        }

        let id = id
            .map(|s| s.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let conn = self.conn();
        conn.execute(
            "INSERT INTO participants (id, name, type, display_name, transport, transport_config, intro, role, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'online')",
            params![id, name, participant_type, display_name, transport, transport_config, intro, role],
        )?;
        get_participant_row(&conn, &id)
    }

    /// 确保默认人类参与者 me 存在（transport=popup，触发审批弹窗）
    #[allow(dead_code)]
    pub fn ensure_default_human(&self) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO participants (id, name, type, display_name, transport, transport_config, intro, status)
             VALUES ('me', 'me', 'human', 'Me', 'popup', '{}', 'default human', 'online')",
            [],
        )?;
        // 兼容旧数据：若 me 已存在（旧 transport=terminal），强制修正为 human/popup
        conn.execute(
            "UPDATE participants SET type='human', transport='popup', intro='default human' WHERE name='me'",
            [],
        )?;
        Ok(())
    }

    /// 确保默认人类参与者 human 存在（transport=popup，触发审批弹窗 / GUI Inbox 显示）。
    ///
    /// `human` 是 `agtalk human` 命令的投递目标，daemon 启动时必须保证该行存在。
    pub fn ensure_human_participant(&self) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO participants (id, name, type, display_name, transport, transport_config, intro, status)
             VALUES ('human', 'human', 'human', 'Human', 'popup', '{}', 'default human', 'online')",
            [],
        )?;
        // 兼容旧数据：若 human 已存在，强制修正为 human/popup
        conn.execute(
            "UPDATE participants SET type='human', transport='popup', intro='default human' WHERE name='human'",
            [],
        )?;
        Ok(())
    }

    pub fn unregister_participant(&self, name: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM participants WHERE name = ?1", params![name])?;
        Ok(())
    }

    /// participant 重新 join 时更新其元数据
    pub fn update_participant_on_join(
        &self,
        name: &str,
        role: &str,
        intro: &str,
        transport: &str,
    ) -> Result<()> {
        self.conn().execute(
            "UPDATE participants
             SET role = ?1, intro = ?2, transport = ?3, status = 'online', last_seen_at = unixepoch('subsec')
             WHERE name = ?4",
            params![role, intro, transport, name],
        )?;
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
            Some(t) => stmt
                .query_map(params![t], row_to_participant)?
                .filter_map(|r| r.ok())
                .collect(),
            None => stmt
                .query_map([], row_to_participant)?
                .filter_map(|r| r.ok())
                .collect(),
        };
        Ok(rows)
    }

    fn list_active_sessions_for_participant(&self, participant_id: &str) -> Result<Vec<SessionInfo>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.workspace_id, s.participant_id, p.name, s.status, s.notify_config
             FROM agent_sessions s JOIN participants p ON s.participant_id = p.id
             WHERE s.participant_id = ?1 AND s.status = 'active'",
        )?;
        let sessions: Vec<SessionInfo> = stmt
            .query_map(params![participant_id], |r| {
                Ok(SessionInfo {
                    session_id: r.get(0)?,
                    workspace_id: r.get(1)?,
                    participant_id: r.get(2)?,
                    participant_name: r.get(3)?,
                    status: r.get(4)?,
                    notify_config: r.get(5)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(sessions)
    }

    pub fn list_peers(&self, participant_type: Option<&str>) -> Result<Vec<PeerInfo>> {
        let participants = self.list_participants(participant_type)?;
        let mut result = Vec::with_capacity(participants.len());
        for p in participants {
            // 注意：list_active_sessions_for_participant 也会获取 self.conn()，
            // 所以这里必须在每次循环内分别获取锁，避免嵌套死锁。
            let sessions = self.list_active_sessions_for_participant(&p.id)?;
            let conn = self.conn();

            // unread / pending 计数
            let (unread, pending): (u32, u32) = conn
                .query_row(
                    "SELECT COALESCE(SUM(CASE WHEN status = 'read' THEN 0 ELSE 1 END), 0),
                            COALESCE(SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END), 0)
                     FROM message_recipients
                     WHERE recipient_id = ?1 AND done_at IS NULL",
                    params![p.id],
                    |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, i64>(1)? as u32)),
                )
                .unwrap_or((0, 0));

            // 最新发送时间
            let last_sent_at: Option<f64> = conn
                .query_row(
                    "SELECT MAX(created_at) FROM messages WHERE sender_id = ?1",
                    params![p.id],
                    |r| r.get(0),
                )
                .ok();

            // 最新读取时间
            let last_read_at: Option<f64> = conn
                .query_row(
                    "SELECT MAX(read_at) FROM message_recipients WHERE recipient_id = ?1",
                    params![p.id],
                    |r| r.get(0),
                )
                .ok();

            result.push(PeerInfo {
                participant: p,
                sessions,
                unread,
                pending,
                last_sent_at,
                last_read_at,
            });
        }
        Ok(result)
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
    #[allow(clippy::too_many_arguments)]
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
        self.send_message_with_attachments(
            sender_name,
            to_names,
            body,
            content_type,
            reply_to,
            conversation_id,
            correlation_id,
            conversation_kind,
            metadata,
            &[],
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send_message_with_attachments(
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
        external_attachments: &[crate::ipc::SendAttachment],
    ) -> Result<Message> {
        let conn = self.conn();
        let sender = get_participant_row(&conn, sender_name)?;
        let sender_id = &sender.id;

        let conv_id = match conversation_id {
            Some(cid) => cid.to_string(),
            None => {
                self.find_or_create_conversation(&conn, sender_id, to_names, conversation_kind)?
            }
        };

        let msg_id = uuid::Uuid::new_v4().to_string();
        let metadata_val = metadata.unwrap_or("{}");

        // 判断是否需要拆附件
        let body_bytes = body.len();
        let stored_body = if body_bytes > self.config.message.attachment_threshold_bytes {
            let preview_chars = self.config.message.preview_limit_chars;
            truncate_chars(body, preview_chars)
        } else {
            body.to_string()
        };

        conn.execute(
            "INSERT INTO messages (id, conversation_id, sender_id, body, content_type, reply_to_id, correlation_id, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![msg_id, conv_id, sender_id, stored_body, content_type, reply_to, correlation_id, metadata_val],
        )?;

        for to_name in to_names {
            if let Ok(recipient) = get_participant_row(&conn, to_name) {
                conn.execute(
                    "INSERT INTO message_recipients (message_id, recipient_id) VALUES (?1, ?2)",
                    params![msg_id, recipient.id],
                )?;
            }
        }

        // 持久化内部 full_body 附件
        if body_bytes > self.config.message.attachment_threshold_bytes {
            let att = self.create_attachment(
                &msg_id,
                "full_body",
                &format!("message-{}.md", msg_id),
                "text/markdown",
                body.as_bytes(),
            )?;
            conn.execute(
                "INSERT INTO attachments (id, message_id, role, filename, content_type, size, storage_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![att.id, att.message_id, att.role, att.filename, att.content_type, att.size as i64, att.filename],
            )?;
        }

        // 外部附件：不复制文件，直接记录原始绝对路径
        for ext in external_attachments {
            let att_id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO attachments (id, message_id, role, filename, content_type, size, storage_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![att_id, msg_id, "attachment", ext.filename, ext.content_type, ext.size as i64, ext.path],
            )?;
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
        let attachments = self.get_attachments_for_msg(&conn, &msg_id)?;
        Ok(Message {
            id: msg_id,
            conversation_id: conv_id,
            sender_id: sender.id.clone(),
            sender_name: sender.name.clone(),
            body: stored_body,
            body_size: body_bytes,
            content_type: content_type.to_string(),
            metadata: metadata_val.to_string(),
            correlation_id: correlation_id.map(|s| s.to_string()),
            status: "pending".to_string(),
            reply_to_id: reply_to.map(|s| s.to_string()),
            recipients,
            attachments,
            full_body: if body_bytes > self.config.message.attachment_threshold_bytes {
                Some(body.to_string())
            } else {
                None
            },
            created_at,
        })
    }

    fn create_attachment(
        &self,
        message_id: &str,
        role: &str,
        _filename: &str,
        content_type: &str,
        data: &[u8],
    ) -> Result<Attachment> {
        let att_id = uuid::Uuid::new_v4().to_string();
        let dir = self
            .config
            .attachment_dir()
            .with_context(|| "无法解析附件目录")?;
        std::fs::create_dir_all(&dir)?;
        // 实际存储文件名使用 message_id + attachment_id，确保唯一且安全
        let storage_filename = format!("{}-{}", message_id, att_id);
        let path = dir.join(&storage_filename);
        std::fs::write(&path, data).with_context(|| format!("无法写入附件文件: {:?}", path))?;
        Ok(Attachment {
            id: att_id,
            message_id: message_id.to_string(),
            role: role.to_string(),
            filename: storage_filename.clone(),
            content_type: content_type.to_string(),
            size: data.len(),
            storage_path: storage_filename,
            created_at: unixepoch_now(),
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
            Some(before) => stmt
                .query_map(params![conversation_id, before, limit], row_to_message)?
                .filter_map(|r| r.ok())
                .collect(),
            None => stmt
                .query_map(params![conversation_id, limit], row_to_message)?
                .filter_map(|r| r.ok())
                .collect(),
        };

        let mut messages = Vec::new();
        for mut msg in rows {
            msg.recipients = self.get_recipients_for_msg(&conn, &msg.id)?;
            msg.attachments = self.get_attachments_for_msg(&conn, &msg.id)?;
            messages.push(msg);
        }
        messages.reverse();
        Ok(messages)
    }

    /// 按 id 查单条消息（自动标记已读）
    pub fn get_message_by_id(
        &self,
        msg_id: &str,
        participant_name: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<Option<Message>> {
        let mut msg = {
            let conn = self.conn();
            let Some(resolved_msg_id) = resolve_message_id(&conn, msg_id)? else {
                return Ok(None);
            };
            let sql = concat!(
                "SELECT m.*, p.name as sname FROM messages m ",
                "JOIN participants p ON m.sender_id = p.id ",
                "WHERE m.id = ?1"
            );
            let mut stmt = conn.prepare(sql)?;
            let mut rows = stmt.query_map(params![resolved_msg_id], row_to_message)?;
            let mut msg = match rows.next() {
                Some(Ok(m)) => m,
                _ => return Ok(None),
            };
            msg.recipients = self.get_recipients_for_msg(&conn, &msg.id)?;
            msg.attachments = self.get_attachments_for_msg(&conn, &msg.id)?;
            // 如果存在 full_body 附件，回填内容
            msg.full_body = self.load_full_body(&conn, &msg.id)?;
            msg
        };

        // 自动标记已读（在释放 conn lock 后调用，避免死锁）
        if let Some(pname) = participant_name {
            let _ = self.mark_read(&msg.id, pname, session_id);
            // 刷新当前消息的 recipient 状态
            msg.recipients = self.get_recipients_for_msg_by_id(&msg.id)?;
        }

        Ok(Some(msg))
    }

    /// 查询指定 approval_request 的回复消息（content_type = approval_response 且 reply_to_id = msg_id）。
    /// 若找到多条，返回最新一条。
    pub fn get_approval_response(&self, msg_id: &str) -> Result<Option<Message>> {
        let conn = self.conn();
        let resolved_msg_id = match resolve_message_id(&conn, msg_id)? {
            Some(id) => id,
            None => return Ok(None),
        };
        let sql = concat!(
            "SELECT m.*, p.name as sname FROM messages m ",
            "JOIN participants p ON m.sender_id = p.id ",
            "WHERE m.content_type = 'approval_response' AND m.reply_to_id = ?1 ",
            "ORDER BY m.created_at DESC LIMIT 1"
        );
        let mut stmt = conn.prepare(sql)?;
        let mut rows = stmt.query_map(params![resolved_msg_id], row_to_message)?;
        let mut msg = match rows.next() {
            Some(Ok(m)) => m,
            _ => return Ok(None),
        };
        msg.recipients = self.get_recipients_for_msg(&conn, &msg.id)?;
        msg.attachments = self.get_attachments_for_msg(&conn, &msg.id)?;
        msg.full_body = self.load_full_body(&conn, &msg.id)?;
        Ok(Some(msg))
    }

    fn load_full_body(&self, conn: &Connection, msg_id: &str) -> Result<Option<String>> {
        let mut stmt = conn.prepare(
            "SELECT storage_path FROM attachments WHERE message_id = ?1 AND role = 'full_body' LIMIT 1"
        )?;
        let mut rows = stmt.query_map(params![msg_id], |row| row.get::<_, String>(0))?;
        if let Some(Ok(storage_path)) = rows.next() {
            let path = self.resolve_attachment_path(&storage_path)?;
            if path.exists() {
                let content = std::fs::read_to_string(&path)
                    .with_context(|| format!("无法读取附件: {:?}", path))?;
                return Ok(Some(content));
            }
        }
        Ok(None)
    }

    /// 解析附件真实路径：绝对路径直接使用，相对路径拼接 attachment_dir。
    fn resolve_attachment_path(&self, storage_path: &str) -> Result<std::path::PathBuf> {
        let p = std::path::Path::new(storage_path);
        if p.is_absolute() {
            Ok(p.to_path_buf())
        } else {
            let dir = self.config.attachment_dir()?;
            Ok(dir.join(storage_path))
        }
    }

    pub fn get_attachment(
        &self,
        attachment_id: &str,
        participant_name: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<Option<(Attachment, Vec<u8>)>> {
        let att = {
            let conn = self.conn();
            let resolved_id = resolve_attachment_id(&conn, attachment_id)?;
            let mut stmt = conn.prepare(
                "SELECT id, message_id, role, filename, content_type, size, storage_path, created_at FROM attachments WHERE id = ?1"
            )?;
            let mut rows = stmt.query_map(params![resolved_id], |row| {
                Ok(Attachment {
                    id: row.get(0)?,
                    message_id: row.get(1)?,
                    role: row.get(2)?,
                    filename: row.get(3)?,
                    content_type: row.get(4)?,
                    size: row.get::<_, i64>(5)? as usize,
                    storage_path: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })?;
            match rows.next() {
                Some(Ok(a)) => a,
                _ => return Ok(None),
            }
        };
        let path = self.resolve_attachment_path(&att.storage_path)?;
        let data = std::fs::read(&path).with_context(|| format!("无法读取附件文件: {:?}", path))?;

        // 读取附件时把对应消息标为已读（注意：必须在释放 conn lock 后调用，避免死锁）
        if let Some(pname) = participant_name {
            let _ = self.mark_read(&att.message_id, pname, session_id);
        }

        Ok(Some((att, data)))
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
            Some(name) => stmt
                .query_map(params![name], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect(),
            None => stmt
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect(),
        };

        let mut conversations = Vec::new();
        for (id, title, kind, created_at, updated_at) in rows {
            let peers = self.get_conv_participant_names(&conn, &id)?;
            let (last_message, counts) =
                self.get_conversation_summary(&conn, &id, participant_name)?;
            conversations.push(Conversation {
                id,
                title,
                kind,
                peers,
                last_message,
                counts,
                created_at,
                updated_at,
            });
        }
        Ok(conversations)
    }

    pub fn mark_done(
        &self,
        msg_id: &str,
        participant_name: &str,
        session_id: Option<&str>,
        external_attachments: &[crate::ipc::SendAttachment],
    ) -> Result<()> {
        let conn = self.conn();
        let resolved_id = resolve_message_id(&conn, msg_id)?
            .ok_or_else(|| anyhow::anyhow!("消息不存在: {}", msg_id))?;
        let p = get_participant_row(&conn, participant_name)?;
        let now = unixepoch_now();
        conn.execute(
            "UPDATE message_recipients 
             SET status = 'done', done_at = ?3, done_by_session_id = ?4,
                 read_at = COALESCE(read_at, ?3), read_by_session_id = COALESCE(read_by_session_id, ?4)
             WHERE message_id = ?1 AND recipient_id = ?2",
            params![resolved_id, p.id, now, session_id],
        )?;

        // 标记完成时可附带外部文件，直接记录原始路径
        for ext in external_attachments {
            let att_id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO attachments (id, message_id, role, filename, content_type, size, storage_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![att_id, resolved_id, "attachment", ext.filename, ext.content_type, ext.size as i64, ext.path],
            )?;
        }

        Ok(())
    }

    pub fn mark_read(
        &self,
        msg_id: &str,
        participant_name: &str,
        session_id: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn();
        let p = get_participant_row(&conn, participant_name)?;
        let now = unixepoch_now();
        conn.execute(
            "UPDATE message_recipients 
             SET status = CASE WHEN status = 'done' THEN 'done' ELSE 'read' END,
                 read_at = COALESCE(read_at, ?3),
                 read_by_session_id = COALESCE(read_by_session_id, ?4)
             WHERE message_id = ?1 AND recipient_id = ?2",
            params![msg_id, p.id, now, session_id],
        )?;
        Ok(())
    }

    pub fn mark_messages_read(
        &self,
        msg_ids: &[String],
        participant_name: &str,
        session_id: Option<&str>,
    ) -> Result<()> {
        if msg_ids.is_empty() {
            return Ok(());
        }
        let conn = self.conn();
        let p = get_participant_row(&conn, participant_name)?;
        let now = unixepoch_now();
        let placeholders: Vec<String> = msg_ids.iter().map(|_| "?".to_string()).collect();
        let sql = format!(
            "UPDATE message_recipients 
             SET status = CASE WHEN status = 'done' THEN 'done' ELSE 'read' END,
                 read_at = COALESCE(read_at, ?),
                 read_by_session_id = COALESCE(read_by_session_id, ?)
             WHERE message_id IN ({}) AND recipient_id = ?",
            placeholders.join(",")
        );
        let mut params: Vec<&dyn rusqlite::ToSql> = Vec::new();
        params.push(&now);
        params.push(&session_id);
        for id in msg_ids {
            params.push(id);
        }
        params.push(&p.id);
        conn.execute(&sql, params.as_slice())?;
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

    pub fn list_inbox(
        &self,
        participant: &str,
        filter: Option<&str>,
        limit: u32,
    ) -> Result<Vec<InboxItem>> {
        let conn = self.conn();

        let (p_id, _p_type) = conn.query_row(
            "SELECT id, type FROM participants WHERE name = ?1 OR id = ?1",
            params![participant],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?;

        let where_clause = match filter {
            Some("unread") => "AND mr.status IN ('pending', 'delivered')",
            Some("pending") => "AND mr.status = 'pending'",
            Some("action_required") => "AND m.content_type IN ('approval_request', 'question', 'task') AND mr.status != 'done'",
            Some("all") | None => "AND mr.status != 'done'",
            _ => "AND mr.status != 'done'",
        };

        let sql = format!(
            "SELECT mr.message_id, mr.status, mr.delivered_at, mr.read_at, mr.done_at, \
                    mr.read_by_session_id, mr.done_by_session_id, \
                    m.sender_id, p2.name as sender_name, p2.type as sender_type, p2.intro as sender_intro, \
                    m.body, m.content_type, m.metadata \
             FROM message_recipients mr \
             JOIN messages m ON mr.message_id = m.id \
             JOIN participants p2 ON m.sender_id = p2.id \
             WHERE mr.recipient_id = ?1 {} \
             ORDER BY (m.content_type IN ('approval_request', 'question', 'task')) DESC, m.created_at DESC \
             LIMIT ?2",
            where_clause
        );

        let inline_limit = self.config.message.inbox_inline_limit_bytes;
        let preview_chars = self.config.message.preview_limit_chars;
        let attachment_threshold = self.config.message.attachment_threshold_bytes;

        #[derive(Debug)]
        struct InboxRow {
            msg_id: String,
            status: String,
            delivered_at: Option<f64>,
            read_at: Option<f64>,
            done_at: Option<f64>,
            read_by_session_id: Option<String>,
            done_by_session_id: Option<String>,
            sender_id: String,
            sender_name: String,
            sender_type: String,
            sender_intro: String,
            body: String,
            content_type: String,
            metadata: String,
        }

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![p_id, limit], |row| {
            Ok(InboxRow {
                msg_id: row.get(0)?,
                status: row.get(1)?,
                delivered_at: row.get(2)?,
                read_at: row.get(3)?,
                done_at: row.get(4)?,
                read_by_session_id: row.get(5)?,
                done_by_session_id: row.get(6)?,
                sender_id: row.get(7)?,
                sender_name: row.get(8)?,
                sender_type: row.get(9)?,
                sender_intro: row.get(10)?,
                body: row.get(11)?,
                content_type: row.get(12)?,
                metadata: row.get(13)?,
            })
        })?;

        let mut raw_items = Vec::new();
        for r in rows.flatten() {
            raw_items.push(r);
        }

        let mut items = Vec::new();
        for row in raw_items {
            let attachments = self.get_attachments_for_msg(&conn, &row.msg_id)?;
            let body_bytes = row.body.len();
            let original_size = attachments
                .iter()
                .find(|a| a.role == "full_body")
                .map(|a| a.size)
                .unwrap_or(body_bytes);

            let has_full_body_attachment = attachments.iter().any(|a| a.role == "full_body");
            let (mode, preview, truncated) = if has_full_body_attachment {
                // 正文已被拆为附件，inbox 只展示摘要
                ("summary".to_string(), row.body.clone(), true)
            } else if original_size <= inline_limit {
                ("full".to_string(), row.body.clone(), false)
            } else if original_size <= attachment_threshold {
                (
                    "preview".to_string(),
                    truncate_chars(&row.body, preview_chars),
                    true,
                )
            } else {
                ("summary".to_string(), row.body.clone(), true)
            };

            let inbox_attachments: Vec<InboxAttachment> = attachments
                .iter()
                .map(|a| InboxAttachment {
                    id: a.id.clone(),
                    role: a.role.clone(),
                    filename: a.filename.clone(),
                    content_type: a.content_type.clone(),
                    size: a.size,
                })
                .collect();

            let (kind, priority, action_required) = match row.content_type.as_str() {
                "approval_request" => ("approval".to_string(), "high".to_string(), true),
                "question" => ("question".to_string(), "high".to_string(), true),
                "task" => ("task".to_string(), "high".to_string(), true),
                _ => ("message".to_string(), "normal".to_string(), false),
            };

            let actions = derive_actions(&row.content_type, &row.status, &inbox_attachments);

            items.push(InboxItem {
                id: row.msg_id.clone(),
                from: InboxSender {
                    id: row.sender_id,
                    name: row.sender_name,
                    kind: row.sender_type,
                    intro: row.sender_intro,
                },
                subject: subject_from_metadata(&row.metadata),
                content: InboxMessageContent {
                    mode,
                    body: preview,
                    truncated,
                    size: original_size,
                },
                attachments: inbox_attachments,
                delivery: InboxDelivery {
                    status: row.status,
                    delivered_at: row.delivered_at,
                    read_at: row.read_at,
                    done_at: row.done_at,
                    read_by_session_id: row.read_by_session_id,
                    done_by_session_id: row.done_by_session_id,
                },
                actions,
                action_required,
                priority,
                kind,
            });
        }
        Ok(items)
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

    pub fn get_recipients_for_msg_by_id(&self, msg_id: &str) -> Result<Vec<RecipientStatus>> {
        let conn = self.conn();
        self.get_recipients_for_msg(&conn, msg_id)
    }

    fn get_recipients_for_msg(
        &self,
        conn: &Connection,
        msg_id: &str,
    ) -> Result<Vec<RecipientStatus>> {
        let mut stmt = conn.prepare(
            "SELECT mr.recipient_id, p.name, mr.status, mr.delivered_at, mr.read_at, mr.done_at, mr.read_by_session_id, mr.done_by_session_id
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
                done_at: row.get(5)?,
                read_by_session_id: row.get(6)?,
                done_by_session_id: row.get(7)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    fn get_attachments_for_msg(&self, conn: &Connection, msg_id: &str) -> Result<Vec<Attachment>> {
        let mut stmt = conn.prepare(
            "SELECT id, message_id, role, filename, content_type, size, storage_path, created_at
             FROM attachments
             WHERE message_id = ?1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map(params![msg_id], |row| {
            Ok(Attachment {
                id: row.get(0)?,
                message_id: row.get(1)?,
                role: row.get(2)?,
                filename: row.get(3)?,
                content_type: row.get(4)?,
                size: row.get::<_, i64>(5)? as usize,
                storage_path: row.get(6)?,
                created_at: row.get(7)?,
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
    ) -> Result<(Option<MessagePreview>, MessageCounts)> {
        let last = conn
            .query_row(
                "SELECT m.id, p.name, m.body, m.created_at
             FROM messages m JOIN participants p ON m.sender_id = p.id
             WHERE m.conversation_id = ?1 ORDER BY m.created_at DESC LIMIT 1",
                params![conv_id],
                |row| {
                    Ok(MessagePreview {
                        id: row.get(0)?,
                        sender_name: row.get(1)?,
                        body: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                },
            )
            .ok();

        let counts = match viewer {
            Some(name) => {
                let unread: u32 = conn.query_row(
                    "SELECT COUNT(*) FROM message_recipients mr
                     JOIN messages m ON mr.message_id = m.id
                     JOIN participants p ON mr.recipient_id = p.id
                     WHERE m.conversation_id = ?1 AND p.name = ?2 AND mr.status IN ('pending','delivered')",
                    params![conv_id, name],
                    |row| row.get(0),
                ).unwrap_or(0);
                let pending: u32 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM message_recipients mr
                     JOIN messages m ON mr.message_id = m.id
                     JOIN participants p ON mr.recipient_id = p.id
                     WHERE m.conversation_id = ?1 AND p.name = ?2 AND mr.status = 'pending'",
                        params![conv_id, name],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
                MessageCounts { unread, pending }
            }
            None => MessageCounts {
                unread: 0,
                pending: 0,
            },
        };

        Ok((last, counts))
    }
}

fn get_participant_row(conn: &Connection, name_or_id: &str) -> Result<Participant> {
    conn.query_row(
        "SELECT * FROM participants WHERE name = ?1 OR id = ?1",
        params![name_or_id],
        row_to_participant,
    )
    .map_err(|_| anyhow::anyhow!("参与者不存在: {}", name_or_id))
}

fn get_participant_by_name_impl(conn: &Connection, name: &str) -> Result<Option<Participant>> {
    match conn.query_row(
        "SELECT * FROM participants WHERE name = ?1",
        params![name],
        row_to_participant,
    ) {
        Ok(p) => Ok(Some(p)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn resolve_attachment_id(conn: &Connection, attachment_id: &str) -> Result<String> {
    match conn.query_row(
        "SELECT id FROM attachments WHERE id = ?1",
        params![attachment_id],
        |row| row.get::<_, String>(0),
    ) {
        Ok(id) => return Ok(id),
        Err(rusqlite::Error::QueryReturnedNoRows) => {}
        Err(e) => return Err(e.into()),
    }

    let pattern = format!("{}%", attachment_id);
    let mut stmt = conn.prepare("SELECT id FROM attachments WHERE id LIKE ?1 LIMIT 2")?;
    let ids: Vec<String> = stmt
        .query_map(params![pattern], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<_, _>>()?;

    match ids.len() {
        0 => Err(anyhow::anyhow!("附件不存在: {}", attachment_id)),
        1 => Ok(ids.into_iter().next().unwrap()),
        _ => Err(anyhow::anyhow!("附件 ID 前缀不唯一: {}", attachment_id)),
    }
}

fn resolve_message_id(conn: &Connection, msg_id: &str) -> Result<Option<String>> {
    match conn.query_row(
        "SELECT id FROM messages WHERE id = ?1",
        params![msg_id],
        |row| row.get::<_, String>(0),
    ) {
        Ok(id) => return Ok(Some(id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => {}
        Err(e) => return Err(e.into()),
    }

    let pattern = format!("{}%", msg_id);
    let mut stmt = conn.prepare("SELECT id FROM messages WHERE id LIKE ?1 LIMIT 2")?;
    let ids: Vec<String> = stmt
        .query_map(params![pattern], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<_, _>>()?;

    match ids.len() {
        0 => Ok(None),
        1 => Ok(ids.into_iter().next()),
        _ => Err(anyhow::anyhow!("消息 ID 前缀不唯一: {}", msg_id)),
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
        intro: row.get("intro")?,
        role: row.get("role")?,
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
        body_size: row.get::<_, String>("body")?.len(),
        content_type: row.get("content_type")?,
        correlation_id: row.get("correlation_id")?,
        metadata: row.get::<_, String>("metadata").unwrap_or_default(),
        status: row.get("status")?,
        reply_to_id: row.get("reply_to_id")?,
        recipients: Vec::new(),
        attachments: Vec::new(),
        full_body: None,
        created_at: row.get("created_at")?,
    })
}

fn unixepoch_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push('…');
    out
}

fn derive_actions(
    content_type: &str,
    status: &str,
    attachments: &[InboxAttachment],
) -> Vec<String> {
    let mut actions = vec!["detail".to_string(), "reply".to_string()];
    if status != "done" {
        actions.push("done".to_string());
    }
    if content_type == "approval_request" && status != "done" {
        actions.push("approve".to_string());
        actions.push("reject".to_string());
    }
    if attachments.iter().any(|a| a.role == "full_body")
        && !actions.contains(&"attachment".to_string())
    {
        actions.push("attachment".to_string());
    }
    actions
}

/// 从 notify_config JSON 中提取稳定的 endpoint key。
///
/// 形如：`<plugin>:<session>:<pane_id>`，例如 `zellij:agtalk-office:1`。
/// 没有 endpoint 信息时返回空字符串，避免 shell / 无 endpoint 的 session 被折叠进同一个冲突桶。
pub fn endpoint_key_from_notify_config(notify_config: &serde_json::Value) -> String {
    let plugin = notify_config
        .get("plugin")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let endpoint = notify_config.get("endpoint").unwrap_or(&serde_json::Value::Null);
    let session = endpoint.get("session").and_then(|v| v.as_str()).unwrap_or("");
    let pane_id = endpoint.get("pane_id").and_then(|v| v.as_str()).unwrap_or("");
    if session.is_empty() && pane_id.is_empty() {
        String::new()
    } else {
        format!("{}:{}:{}", plugin, session, pane_id)
    }
}

// ─── workspace / session（v4）──────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub workspace_id: String,
    pub participant_id: String,
    pub participant_name: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notify_config: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PeerInfo {
    #[serde(flatten)]
    pub participant: Participant,
    pub sessions: Vec<SessionInfo>,
    pub unread: u32,
    pub pending: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sent_at: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_read_at: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[allow(dead_code)]
pub struct WorkspaceRow {
    pub id: String,
    pub name: String,
    pub root: String,
}

impl Storage {
    /// 注册 workspace（同 root 复用），返回 workspace_id
    pub fn register_workspace(&self, name: &str, root: &str) -> Result<String> {
        let conn = self.conn();
        if let Ok(id) = conn.query_row(
            "SELECT id FROM workspaces WHERE root = ?1",
            params![root],
            |r| r.get(0),
        ) {
            return Ok(id);
        }
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO workspaces (id, name, root) VALUES (?1, ?2, ?3)",
            params![id, name, root],
        )?;
        Ok(id)
    }

    #[allow(dead_code)]
    pub fn get_workspace_by_root(&self, root: &str) -> Result<Option<WorkspaceRow>> {
        let conn = self.conn();
        match conn.query_row(
            "SELECT id, name, root FROM workspaces WHERE root = ?1",
            params![root],
            |r| {
                Ok(WorkspaceRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    root: r.get(2)?,
                })
            },
        ) {
            Ok(w) => Ok(Some(w)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// 创建 session，返回 (session_id, token)
    pub fn create_session(
        &self,
        workspace_id: &str,
        participant_id: &str,
        endpoint_key: &str,
        notify_config: Option<&str>,
        runtime_config: Option<&str>,
    ) -> Result<(String, String)> {
        let conn = self.conn();
        let id = uuid::Uuid::new_v4().to_string();
        let token = format!("agt_{}", uuid::Uuid::new_v4().to_string().replace('-', ""));
        conn.execute(
            "INSERT INTO agent_sessions (id, workspace_id, participant_id, token, status, endpoint_key, notify_config, runtime_config)
             VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?6, ?7)",
            params![id, workspace_id, participant_id, token, endpoint_key, notify_config, runtime_config],
        )?;
        Ok((id, token))
    }

    /// 原子地创建 session 并在需要时接管同 endpoint 旧 session。
    ///
    /// 整个流程在一个 SQLite 事务中完成：创建新 session → 退役同 endpoint 旧 active session。
    /// 若创建失败或事务回滚，旧 session 仍保持 active，满足 takeover 原子性要求。
    /// 返回 (session_id, token, 被退役的旧 session 列表)。
    pub fn create_session_with_takeover(
        &self,
        workspace_id: &str,
        participant_id: &str,
        endpoint_key: &str,
        notify_config: Option<&str>,
        runtime_config: Option<&str>,
    ) -> Result<(String, String, Vec<SessionInfo>)> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;

        let id = uuid::Uuid::new_v4().to_string();
        let token = format!("agt_{}", uuid::Uuid::new_v4().to_string().replace('-', ""));
        tx.execute(
            "INSERT INTO agent_sessions (id, workspace_id, participant_id, token, status, endpoint_key, notify_config, runtime_config)
             VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?6, ?7)",
            params![id, workspace_id, participant_id, token, endpoint_key, notify_config, runtime_config],
        )?;

        // 在同一事务中退役同 endpoint 的其他 active session（新 session 已创建，排除自己）
        let mut stmt = tx.prepare(
            "SELECT s.id, s.workspace_id, s.participant_id, p.name, s.status, s.notify_config
             FROM agent_sessions s JOIN participants p ON s.participant_id = p.id
             WHERE s.workspace_id = ?1 AND s.endpoint_key = ?2 AND s.status = 'active' AND s.id != ?3",
        )?;
        let rows = stmt.query_map(params![workspace_id, endpoint_key, &id], |r| {
            Ok(SessionInfo {
                session_id: r.get(0)?,
                workspace_id: r.get(1)?,
                participant_id: r.get(2)?,
                participant_name: r.get(3)?,
                status: r.get(4)?,
                notify_config: r.get(5)?,
            })
        })?;
        let retired: Vec<SessionInfo> = rows.filter_map(|r| r.ok()).collect();
        drop(stmt);

        tx.execute(
            "UPDATE agent_sessions SET status = 'left' WHERE workspace_id = ?1 AND endpoint_key = ?2 AND status = 'active' AND id != ?3",
            params![workspace_id, endpoint_key, &id],
        )?;

        tx.commit()?;
        Ok((id, token, retired))
    }

    /// 校验 session_id + token，返回绑定身份
    pub fn validate_session(&self, session_id: &str, token: &str) -> Result<Option<SessionInfo>> {
        let conn = self.conn();
        match conn.query_row(
            "SELECT s.id, s.workspace_id, s.participant_id, p.name, s.status, s.notify_config
             FROM agent_sessions s JOIN participants p ON s.participant_id = p.id
             WHERE s.id = ?1 AND s.token = ?2",
            params![session_id, token],
            |r| {
                Ok(SessionInfo {
                    session_id: r.get(0)?,
                    workspace_id: r.get(1)?,
                    participant_id: r.get(2)?,
                    participant_name: r.get(3)?,
                    status: r.get(4)?,
                    notify_config: r.get(5)?,
                })
            },
        ) {
            Ok(info) => Ok(Some(info)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn mark_session_left(&self, session_id: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE agent_sessions SET status = 'left' WHERE id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    /// 检查指定 session 是否仍是 active。
    pub fn is_session_active(&self, session_id: &str) -> Result<bool> {
        let conn = self.conn();
        let status: Option<String> = conn
            .query_row(
                "SELECT status FROM agent_sessions WHERE id = ?1",
                params![session_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(status.as_deref() == Some("active"))
    }

    /// 查询 workspace + endpoint 上的所有 active session。
    pub fn get_active_sessions_by_endpoint(
        &self,
        workspace_id: &str,
        endpoint_key: &str,
    ) -> Result<Vec<SessionInfo>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.workspace_id, s.participant_id, p.name, s.status, s.notify_config
             FROM agent_sessions s JOIN participants p ON s.participant_id = p.id
             WHERE s.workspace_id = ?1 AND s.endpoint_key = ?2 AND s.status = 'active'",
        )?;
        let rows = stmt.query_map(params![workspace_id, endpoint_key], |r| {
            Ok(SessionInfo {
                session_id: r.get(0)?,
                workspace_id: r.get(1)?,
                participant_id: r.get(2)?,
                participant_name: r.get(3)?,
                status: r.get(4)?,
                notify_config: r.get(5)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// 退役 workspace + endpoint 上除 except_session_id 外的所有 active session。
    /// 返回被退役的 session 信息（用于更新本地 session 文件）。
    #[allow(dead_code)]
    pub fn retire_sessions_by_endpoint_except(
        &self,
        workspace_id: &str,
        endpoint_key: &str,
        except_session_id: &str,
    ) -> Result<Vec<SessionInfo>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.workspace_id, s.participant_id, p.name, s.status, s.notify_config
             FROM agent_sessions s JOIN participants p ON s.participant_id = p.id
             WHERE s.workspace_id = ?1 AND s.endpoint_key = ?2 AND s.status = 'active' AND s.id != ?3",
        )?;
        let rows = stmt.query_map(params![workspace_id, endpoint_key, except_session_id], |r| {
            Ok(SessionInfo {
                session_id: r.get(0)?,
                workspace_id: r.get(1)?,
                participant_id: r.get(2)?,
                participant_name: r.get(3)?,
                status: r.get(4)?,
                notify_config: r.get(5)?,
            })
        })?;
        let retired: Vec<SessionInfo> = rows.filter_map(|r| r.ok()).collect();
        conn.execute(
            "UPDATE agent_sessions SET status = 'left' WHERE workspace_id = ?1 AND endpoint_key = ?2 AND status = 'active' AND id != ?3",
            params![workspace_id, endpoint_key, except_session_id],
        )?;
        Ok(retired)
    }

    /// 根据 participant 的 active session 数量重新设置 online/offline。
    pub fn recompute_participant_status(&self, participant_id: &str) -> Result<()> {
        let conn = self.conn();
        let active_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM agent_sessions WHERE participant_id = ?1 AND status = 'active'",
            params![participant_id],
            |r| r.get(0),
        )?;
        let status = if active_count > 0 { "online" } else { "offline" };
        conn.execute(
            "UPDATE participants SET status = ?1 WHERE id = ?2",
            params![status, participant_id],
        )?;
        Ok(())
    }

    /// 清理指定 workspace 下的 inactive session。
    ///
    /// dry_run=true 时仅返回会被清理的 participant 名称；
    /// dry_run=false 时删除该 workspace 下 inactive session，并返回需要删除本地 session 文件的 participant。
    /// 只操作当前 workspace，不会扫描全库。
    pub fn cleanup_inactive_sessions(&self, workspace_id: &str, dry_run: bool) -> Result<Vec<String>> {
        let conn = self.conn();
        // 当前 workspace 下所有 inactive session 的 participant
        let mut stmt = conn.prepare(
            "SELECT DISTINCT s.participant_id, p.name
             FROM agent_sessions s JOIN participants p ON s.participant_id = p.id
             WHERE s.workspace_id = ?1 AND s.status != 'active'",
        )?;
        let rows = stmt.query_map(params![workspace_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let participants: Vec<(String, String)> = rows.filter_map(|r| r.ok()).collect();

        let mut to_clean = Vec::new();
        for (participant_id, name) in participants {
            let active_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM agent_sessions WHERE workspace_id = ?1 AND participant_id = ?2 AND status = 'active'",
                params![workspace_id, participant_id],
                |r| r.get(0),
            )?;
            if active_count == 0 {
                to_clean.push(name);
            }
        }

        if !dry_run {
            conn.execute(
                "DELETE FROM agent_sessions WHERE workspace_id = ?1 AND status != 'active'",
                params![workspace_id],
            )?;
        }

        Ok(to_clean)
    }

    /// 按 id 查询 session 信息。
    pub fn get_session_by_id(&self, session_id: &str) -> Result<Option<SessionInfo>> {
        let conn = self.conn();
        match conn.query_row(
            "SELECT s.id, s.workspace_id, s.participant_id, p.name, s.status, s.notify_config
             FROM agent_sessions s JOIN participants p ON s.participant_id = p.id
             WHERE s.id = ?1",
            params![session_id],
            |r| {
                Ok(SessionInfo {
                    session_id: r.get(0)?,
                    workspace_id: r.get(1)?,
                    participant_id: r.get(2)?,
                    participant_name: r.get(3)?,
                    status: r.get(4)?,
                    notify_config: r.get(5)?,
                })
            },
        ) {
            Ok(info) => Ok(Some(info)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    #[allow(dead_code)]
    pub fn touch_session(&self, session_id: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE agent_sessions SET last_seen_at = unixepoch('subsec') WHERE id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn list_active_sessions(&self, workspace_id: &str) -> Result<Vec<SessionInfo>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.workspace_id, s.participant_id, p.name, s.status, s.notify_config
             FROM agent_sessions s JOIN participants p ON s.participant_id = p.id
             WHERE s.workspace_id = ?1 AND s.status = 'active'",
        )?;
        let rows = stmt.query_map(params![workspace_id], |r| {
            Ok(SessionInfo {
                session_id: r.get(0)?,
                workspace_id: r.get(1)?,
                participant_id: r.get(2)?,
                participant_name: r.get(3)?,
                status: r.get(4)?,
                notify_config: r.get(5)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// 取某个 participant 最新的 active session（用于 notify 投递）
    pub fn get_active_session_for_participant(
        &self,
        participant_id: &str,
    ) -> Result<Option<SessionInfo>> {
        let conn = self.conn();
        match conn.query_row(
            "SELECT s.id, s.workspace_id, s.participant_id, p.name, s.status, s.notify_config
             FROM agent_sessions s JOIN participants p ON s.participant_id = p.id
             WHERE s.participant_id = ?1 AND s.status = 'active'
             ORDER BY s.created_at DESC
             LIMIT 1",
            params![participant_id],
            |r| {
                Ok(SessionInfo {
                    session_id: r.get(0)?,
                    workspace_id: r.get(1)?,
                    participant_id: r.get(2)?,
                    participant_name: r.get(3)?,
                    status: r.get(4)?,
                    notify_config: r.get(5)?,
                })
            },
        ) {
            Ok(info) => Ok(Some(info)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
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

        let storage = Storage {
            conn: Mutex::new(conn),
            config: Arc::new(AgConfig::default()),
        };
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

#[cfg(test)]
mod takeover_tests {
    use super::*;

    fn storage() -> Storage {
        Storage::open_memory().unwrap()
    }

    fn register_alice(storage: &Storage) -> Participant {
        storage
            .register_participant(None, "alice", "agent", "Alice", "terminal", "{}", "", "agent")
            .unwrap()
    }

    fn make_notify(plugin: &str, session: &str, pane_id: &str) -> serde_json::Value {
        serde_json::json!({
            "plugin": plugin,
            "endpoint": { "session": session, "pane_id": pane_id },
            "send_enter": true,
        })
    }

    #[test]
    fn test_create_session_stores_endpoint_key() {
        let s = storage();
        let p = register_alice(&s);
        let ws = s.register_workspace("ws", "/tmp/ws").unwrap();
        let notify = make_notify("zellij", "main", "1");
        let key = endpoint_key_from_notify_config(&notify);
        let (sid, _) = s
            .create_session(&ws, &p.id, &key, Some(&notify.to_string()), None)
            .unwrap();

        let info = s.get_session_by_id(&sid).unwrap().unwrap();
        assert_eq!(info.participant_name, "alice");
        assert_eq!(info.status, "active");
    }

    #[test]
    fn test_same_endpoint_conflict_and_takeover() {
        let s = storage();
        let p = register_alice(&s);
        let ws = s.register_workspace("ws", "/tmp/ws").unwrap();
        let notify = make_notify("zellij", "main", "1");
        let key = endpoint_key_from_notify_config(&notify);

        let (old_sid, _) = s
            .create_session(&ws, &p.id, &key, Some(&notify.to_string()), None)
            .unwrap();

        // 同 endpoint 上应能查到 active session
        let existing = s.get_active_sessions_by_endpoint(&ws, &key).unwrap();
        assert_eq!(existing.len(), 1);
        assert_eq!(existing[0].session_id, old_sid);

        // takeover 会退役旧 session
        let retired = s
            .retire_sessions_by_endpoint_except(&ws, &key, "")
            .unwrap();
        assert_eq!(retired.len(), 1);
        assert_eq!(retired[0].session_id, old_sid);

        // 旧 session 已 left
        assert!(!s.is_session_active(&old_sid).unwrap());
    }

    #[test]
    fn test_recompute_participant_status_after_leave() {
        let s = storage();
        let p = register_alice(&s);
        let ws = s.register_workspace("ws", "/tmp/ws").unwrap();
        let notify1 = make_notify("zellij", "main", "1");
        let key1 = endpoint_key_from_notify_config(&notify1);
        let notify2 = make_notify("zellij", "main", "2");
        let key2 = endpoint_key_from_notify_config(&notify2);

        let (sid1, _) = s
            .create_session(&ws, &p.id, &key1, Some(&notify1.to_string()), None)
            .unwrap();
        let (sid2, _) = s
            .create_session(&ws, &p.id, &key2, Some(&notify2.to_string()), None)
            .unwrap();

        s.update_participant_status("alice", "online").unwrap();
        s.mark_session_left(&sid1).unwrap();
        s.recompute_participant_status(&p.id).unwrap();

        let p = s.get_participant_by_name("alice").unwrap().unwrap();
        assert_eq!(p.status, "online");

        s.mark_session_left(&sid2).unwrap();
        s.recompute_participant_status(&p.id).unwrap();

        let p = s.get_participant_by_name("alice").unwrap().unwrap();
        assert_eq!(p.status, "offline");
    }

    #[test]
    fn test_cleanup_inactive_sessions_only() {
        let s = storage();
        let p = register_alice(&s);
        let ws = s.register_workspace("ws", "/tmp/ws").unwrap();
        let notify = make_notify("zellij", "main", "1");
        let key = endpoint_key_from_notify_config(&notify);

        let (inactive_sid, _) = s
            .create_session(&ws, &p.id, &key, Some(&notify.to_string()), None)
            .unwrap();
        s.mark_session_left(&inactive_sid).unwrap();

        // 干跑返回 alice，但不删除
        let dry = s.cleanup_inactive_sessions(&ws, true).unwrap();
        assert_eq!(dry, vec!["alice"]);
        assert!(s.get_session_by_id(&inactive_sid).unwrap().is_some());

        // 真正清理
        let removed = s.cleanup_inactive_sessions(&ws, false).unwrap();
        assert_eq!(removed, vec!["alice"]);
        assert!(s.get_session_by_id(&inactive_sid).unwrap().is_none());
    }

    #[test]
    fn test_cleanup_keeps_active_session() {
        let s = storage();
        let p = register_alice(&s);
        let ws = s.register_workspace("ws", "/tmp/ws").unwrap();
        let notify = make_notify("zellij", "main", "1");
        let key = endpoint_key_from_notify_config(&notify);

        let (active_sid, _) = s
            .create_session(&ws, &p.id, &key, Some(&notify.to_string()), None)
            .unwrap();

        let removed = s.cleanup_inactive_sessions(&ws, false).unwrap();
        assert!(removed.is_empty());
        assert!(s.is_session_active(&active_sid).unwrap());
    }

    #[test]
    fn test_create_session_with_takeover_atomic() {
        let s = storage();
        let p = register_alice(&s);
        let ws = s.register_workspace("ws", "/tmp/ws").unwrap();
        let notify = make_notify("zellij", "main", "1");
        let key = endpoint_key_from_notify_config(&notify);

        let (old_sid, _) = s
            .create_session(&ws, &p.id, &key, Some(&notify.to_string()), None)
            .unwrap();

        let (new_sid, _, retired) = s
            .create_session_with_takeover(&ws, &p.id, &key, Some(&notify.to_string()), None)
            .unwrap();

        assert_ne!(old_sid, new_sid);
        assert_eq!(retired.len(), 1);
        assert_eq!(retired[0].session_id, old_sid);
        assert!(!s.is_session_active(&old_sid).unwrap());
        assert!(s.is_session_active(&new_sid).unwrap());
    }

    #[test]
    fn test_endpoint_key_empty_for_shell() {
        let shell_notify = serde_json::json!({ "plugin": "terminal" });
        assert_eq!(endpoint_key_from_notify_config(&shell_notify), "");

        let empty_endpoint = serde_json::json!({
            "plugin": "terminal",
            "endpoint": {}
        });
        assert_eq!(endpoint_key_from_notify_config(&empty_endpoint), "");
    }

    #[test]
    fn test_shell_sessions_do_not_conflict() {
        let s = storage();
        let alice = register_alice(&s);
        let bob = s
            .register_participant(None, "bob", "agent", "Bob", "terminal", "{}", "", "agent")
            .unwrap();
        let ws = s.register_workspace("ws", "/tmp/ws").unwrap();
        let shell_notify = serde_json::json!({ "plugin": "terminal" });
        let key = endpoint_key_from_notify_config(&shell_notify);
        assert!(key.is_empty());

        let (sid1, _) = s
            .create_session(&ws, &alice.id, &key, Some(&shell_notify.to_string()), None)
            .unwrap();
        let (sid2, _) = s
            .create_session(&ws, &bob.id, &key, Some(&shell_notify.to_string()), None)
            .unwrap();

        assert!(s.is_session_active(&sid1).unwrap());
        assert!(s.is_session_active(&sid2).unwrap());
    }

    #[test]
    fn test_takeover_failure_keeps_old_session_active() {
        let s = storage();
        let p = register_alice(&s);
        let ws = s.register_workspace("ws", "/tmp/ws").unwrap();
        let notify = make_notify("zellij", "main", "1");
        let key = endpoint_key_from_notify_config(&notify);

        let (old_sid, _) = s
            .create_session(&ws, &p.id, &key, Some(&notify.to_string()), None)
            .unwrap();

        // 用不存在的 participant_id 触发新 session 创建失败，事务必须回滚
        let result = s.create_session_with_takeover(
            &ws,
            "nonexistent-participant-id",
            &key,
            Some(&notify.to_string()),
            None,
        );
        assert!(result.is_err(), "不存在的 participant 应导致创建失败");

        // 旧 session 必须保持 active，不能因 takeover 失败被退役
        assert!(s.is_session_active(&old_sid).unwrap());
    }

    #[test]
    fn test_cleanup_is_workspace_scoped() {
        let s = storage();
        let p = register_alice(&s);
        let ws1 = s.register_workspace("ws1", "/tmp/ws1").unwrap();
        let ws2 = s.register_workspace("ws2", "/tmp/ws2").unwrap();
        let notify = make_notify("zellij", "main", "1");
        let key = endpoint_key_from_notify_config(&notify);

        let (inactive_in_ws1, _) = s
            .create_session(&ws1, &p.id, &key, Some(&notify.to_string()), None)
            .unwrap();
        s.mark_session_left(&inactive_in_ws1).unwrap();

        let (inactive_in_ws2, _) = s
            .create_session(&ws2, &p.id, &key, Some(&notify.to_string()), None)
            .unwrap();
        s.mark_session_left(&inactive_in_ws2).unwrap();

        // 只清理 ws1，ws2 的 inactive session 应保留
        let removed = s.cleanup_inactive_sessions(&ws1, false).unwrap();
        assert_eq!(removed, vec!["alice"]);
        assert!(s.get_session_by_id(&inactive_in_ws1).unwrap().is_none());
        assert!(s.get_session_by_id(&inactive_in_ws2).unwrap().is_some());
    }
}
