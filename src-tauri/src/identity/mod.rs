//! 身份模块：session.json、agents.json、mailbox、认证。

pub mod agents_map;
pub mod auth;
pub mod browser_session;
pub mod history;
pub mod mailbox;
pub mod session_file;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON 错误: {0}")]
    Json(#[from] serde_json::Error),
    #[error("路径错误: {0}")]
    Paths(#[from] crate::paths::PathsError),
    #[error("数据库错误: {0}")]
    Storage(#[from] crate::storage::StorageError),
    #[error("SQLite 错误: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Mailbox 不存在: {0}")]
    MailboxNotFound(String),
    #[error("session 文件不存在或 address 不匹配")]
    SessionMismatch,
    #[error("agent 未在 agents.json 注册")]
    AgentNotRegistered,
    #[error("PID 已复用（start_time 不一致）")]
    PidReused,
    #[error("进程不存在")]
    ProcessNotFound,
    #[error("UUID 解析错误")]
    InvalidUuid,
}
