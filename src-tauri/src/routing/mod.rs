//! 路由模块：send、lookup、inbox、reply。

pub mod inbox;
pub mod lookup;
pub mod reply;
pub mod send;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RoutingError {
    #[error("数据库错误: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("存储层错误: {0}")]
    Storage(#[from] crate::storage::StorageError),
    #[error("身份错误: {0}")]
    Identity(#[from] crate::identity::IdentityError),
    #[error("消息不存在: {0}")]
    MessageNotFound(String),
    #[error("消息 ID 太短: {0}")]
    MessageIdTooShort(String),
    #[error("消息 ID '{short_id}' 匹配到 {count} 条消息，请使用更长前缀或完整 UUID")]
    MessageIdAmbiguous { short_id: String, count: usize },
    #[error("目标 mailbox 不存在: {0}")]
    MailboxNotFound(String),
    #[error("JSON 错误: {0}")]
    Json(#[from] serde_json::Error),
    #[error("事件 ID 分配失败")]
    EventIdAllocation,
}

/// 取消息/UUID 的短 ID：第一个 `-` 之前的部分；无 `-` 则返回原串。
pub fn short_id_of(id: &str) -> String {
    id.split('-').next().unwrap_or(id).to_string()
}

pub struct SendRequest<'a> {
    pub to: &'a str,
    pub to_name: &'a str,
    pub from: &'a str,
    pub from_name: &'a str,
    pub body: &'a str,
    pub content_type: &'a str,
    pub reply_to_id: Option<&'a str>,
    pub metadata: &'a str,
    pub more_coming: bool,
}

#[derive(Debug, Clone)]
pub struct StatusChange {
    pub message_id: String,
    pub old_status: String,
    pub new_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub to_address: String,
    pub to_name: String,
    pub from_address: String,
    pub from_name: String,
    pub body: String,
    pub content_type: String,
    pub reply_to_id: Option<String>,
    pub metadata: String,
    pub event_id: i64,
    pub status: String,
    pub created_at: f64,
}

impl Message {
    pub(crate) fn from_row(row: &rusqlite::Row<'_>) -> Result<Self, rusqlite::Error> {
        Ok(Self {
            id: row.get("id")?,
            to_address: row.get("to_address")?,
            to_name: row.get("to_name")?,
            from_address: row.get("from_address")?,
            from_name: row.get("from_name")?,
            body: row.get("body")?,
            content_type: row.get("content_type")?,
            reply_to_id: row.get("reply_to_id")?,
            metadata: row.get("metadata")?,
            event_id: row.get("event_id")?,
            status: row.get("status")?,
            created_at: row.get("created_at")?,
        })
    }
}
