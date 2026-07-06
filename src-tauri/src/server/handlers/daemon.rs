//! daemon 状态 handler：无需身份认证。

use crate::server::daemon;
use crate::server::state::AppState;
use axum::extract::State;

pub fn handle_status(state: &AppState) -> crate::proto::ServerMsg {
    daemon::build_status(state)
}

/// Axum handler wrapper。
pub async fn daemon_status_handler(
    State(state): State<AppState>,
) -> (
    axum::http::StatusCode,
    axum::response::Json<crate::proto::ServerMsg>,
) {
    let resp = handle_status(&state);
    let status = crate::server::handlers::status_for(&resp);
    (status, axum::response::Json(resp))
}
