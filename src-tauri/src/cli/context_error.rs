//! CLI 身份解析错误：提供稳定 code，方便 agent 判断下一步动作。

use thiserror::Error;

#[derive(Debug, Error)]
pub enum IdentityResolutionError {
    #[error("当前进程未注册身份，且工作目录没有可用 session；先执行 agtalk id join")]
    Required,

    #[error("工作目录有多个 session ({0:?})，请用 --as <name> 或 AGTALK_NAME 指定")]
    Ambiguous(Vec<String>),

    #[error("agents.json 中 pid {pid} 指向的 session '{name}' 已不存在")]
    StaleAnchor { pid: u32, name: String },

    #[error("未找到身份 '{name}' 的 session.json")]
    SessionMissing { name: String },

    #[error("身份 '{name}' 的 session.json 无效: {reason}")]
    InvalidSession { name: String, reason: String },

    #[error("IO 错误: {0}")]
    Io(String),
}

impl IdentityResolutionError {
    /// 稳定错误码，供 --json 输出和 agent 判断。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Required => "identity_required",
            Self::Ambiguous(_) => "identity_ambiguous",
            Self::StaleAnchor { .. } => "identity_stale_anchor",
            Self::SessionMissing { .. } => "identity_session_missing",
            Self::InvalidSession { .. } => "identity_invalid_session",
            Self::Io(_) => "identity_io_error",
        }
    }
}

impl From<std::io::Error> for IdentityResolutionError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}
