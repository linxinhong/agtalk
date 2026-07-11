//! REST API v1 handler 共享工具。

use crate::identity::auth::{self, AuthenticatedSession};
use crate::proto::ServerMsg;
use crate::server::state::AppState;
use axum::http::{HeaderMap, StatusCode};
use std::path::{Path, PathBuf};

pub mod config;
pub mod daemon;
pub mod human;
pub mod id;
pub mod mem;
pub mod msg;
pub mod tool;

/// 从 header 读取 workspace root；缺失时退化为 daemon 启动时的 legacy 目录。
pub fn workspace_root_from_headers(headers: &HeaderMap, fallback: &Path) -> PathBuf {
    headers
        .get("X-AgTalk-Workspace-Root")
        .and_then(|v| v.to_str().ok())
        .map(PathBuf::from)
        .unwrap_or_else(|| fallback.to_path_buf())
}

/// 从请求头读取认证信息并认证。
#[allow(clippy::result_large_err)]
pub fn authenticate_req(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthenticatedSession, ServerMsg> {
    let workspace_root = workspace_root_from_headers(headers, &state.dot_agtalk);
    let address = headers
        .get("X-AgTalk-Address")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| auth_error("缺少 X-AgTalk-Address".into()))?;
    let pid = headers
        .get("X-AgTalk-Pid")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let start_time = headers
        .get("X-AgTalk-Start-Time")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let browser_token = headers
        .get("X-AgTalk-Browser-Token")
        .and_then(|v| v.to_str().ok());

    auth::authenticate(
        &state.storage,
        &workspace_root,
        address,
        pid,
        start_time,
        browser_token,
    )
    .map_err(|e| auth_error(e.to_string()))
}

/// 构造通用认证错误响应。
pub fn auth_error(message: String) -> ServerMsg {
    ServerMsg::Error {
        code: "auth_failed".into(),
        message,
    }
}

/// 构造通用未支持错误响应。
pub fn not_supported(what: &str) -> ServerMsg {
    ServerMsg::Error {
        code: "not_supported".into(),
        message: format!("{} 尚未实现", what),
    }
}

/// 根据 ServerMsg 确定 HTTP 状态码。
pub fn status_for(msg: &ServerMsg) -> StatusCode {
    match msg {
        ServerMsg::Error { code, .. } if code == "auth_failed" => StatusCode::UNAUTHORIZED,
        ServerMsg::Error { .. } => StatusCode::BAD_REQUEST,
        _ => StatusCode::OK,
    }
}

/// 读取可选的 browser token（不认证）。
pub fn browser_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("X-AgTalk-Browser-Token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}
