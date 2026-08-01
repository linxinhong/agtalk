//! REST API v1 handler 共享工具。

use crate::identity::auth::{self, AuthenticatedSession};
use crate::proto::ServerMsg;
use crate::server::state::AppState;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;
use std::path::PathBuf;

pub mod config;
pub mod daemon;
pub mod graph;
pub mod graph_dispatch;
#[cfg(test)]
pub mod graph_tests;
pub mod graph_verify;
pub mod http_human;
pub mod http_id;
pub mod http_mem;
pub mod http_msg;
pub mod http_tool;
pub mod human;
pub mod id;
pub mod mem;
pub mod msg;
pub mod tool;

/// 从 Agent 请求头读取 `.agtalk` 根目录。
///
/// 不允许回退到 daemon 启动目录，否则其它项目的 agent session 会被错误写入 daemon 项目。
#[allow(clippy::result_large_err)]
pub fn workspace_root_from_headers(headers: &HeaderMap) -> Result<PathBuf, ServerMsg> {
    let raw = headers
        .get("X-AgTalk-Workspace-Root")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ServerMsg::Error {
            code: "workspace_root_required".to_string(),
            message: "缺少 X-AgTalk-Workspace-Root；请在目标项目目录执行 agtalk".to_string(),
        })?;
    let root = PathBuf::from(raw);
    let is_dot_agtalk = root.file_name().and_then(|v| v.to_str()) == Some(".agtalk");
    if !root.is_absolute() || !is_dot_agtalk {
        return Err(ServerMsg::Error {
            code: "workspace_root_invalid".to_string(),
            message: "X-AgTalk-Workspace-Root 必须是绝对的 .agtalk 目录".to_string(),
        });
    }
    Ok(root)
}

/// 从请求头读取认证信息并认证。
#[allow(clippy::result_large_err)]
pub fn authenticate_req(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthenticatedSession, ServerMsg> {
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
    // 浏览器扩展由 token 认证，workspace 由 browser_session 决定；CLI agent 必须显式携带当前目录。
    let workspace_root = if browser_token.is_some() {
        state.dot_agtalk.clone()
    } else {
        workspace_root_from_headers(headers)?
    };

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_root_requires_header() {
        let err = workspace_root_from_headers(&HeaderMap::new()).unwrap_err();
        assert!(
            matches!(err, ServerMsg::Error { ref code, .. } if code == "workspace_root_required")
        );
    }

    #[test]
    fn workspace_root_rejects_non_absolute_or_non_dot_agtalk_path() {
        for value in [".agtalk", "/tmp/not-agtalk", "relative/.agtalk"] {
            let mut headers = HeaderMap::new();
            headers.insert("X-AgTalk-Workspace-Root", value.parse().unwrap());
            let err = workspace_root_from_headers(&headers).unwrap_err();
            assert!(
                matches!(err, ServerMsg::Error { ref code, .. } if code == "workspace_root_invalid")
            );
        }
    }

    #[test]
    fn workspace_root_accepts_absolute_dot_agtalk_path() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-AgTalk-Workspace-Root",
            "/tmp/project/.agtalk".parse().unwrap(),
        );
        assert_eq!(
            workspace_root_from_headers(&headers).unwrap(),
            PathBuf::from("/tmp/project/.agtalk")
        );
    }
}

/// HTTP JSON 响应包装（axum handler 层共享）。
pub(crate) fn json_response(resp: ServerMsg) -> (StatusCode, Json<ServerMsg>) {
    let status = status_for(&resp);
    (status, Json(resp))
}
