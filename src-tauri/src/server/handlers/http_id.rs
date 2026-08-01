use super::id;
use super::json_response;
use crate::proto::ServerMsg;
use crate::server::state::AppState;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;
use std::collections::HashMap;

// ---- id ----

#[derive(serde::Deserialize)]
pub struct IdJoinBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    intro: Option<String>,
    #[serde(default = "crate::proto::default_notify")]
    notify: String,
    #[serde(default)]
    notify_endpoint: Option<serde_json::Value>,
    pid: u32,
    start_time: u64,
}

pub async fn id_join_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<IdJoinBody>,
) -> (StatusCode, Json<ServerMsg>) {
    let workspace_root = match crate::server::handlers::workspace_root_from_headers(&headers) {
        Ok(root) => root,
        Err(error) => return json_response(error),
    };
    json_response(id::handle_join(
        &state,
        &workspace_root,
        body.name,
        body.intro,
        body.notify,
        body.notify_endpoint,
        body.pid,
        body.start_time,
    ))
}

#[derive(serde::Deserialize)]
pub struct IdLeaveBody {
    #[serde(default)]
    purge: bool,
    #[serde(default)]
    address: Option<String>,
}

pub async fn id_leave_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<IdLeaveBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(id::handle_leave(&state, &headers, body.address, body.purge))
}

#[derive(serde::Deserialize)]
pub struct IdCleanupBody {
    #[serde(default)]
    execute: bool,
}

pub async fn id_cleanup_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<IdCleanupBody>,
) -> (StatusCode, Json<ServerMsg>) {
    let workspace_root = match crate::server::handlers::workspace_root_from_headers(&headers) {
        Ok(root) => root,
        Err(error) => return json_response(error),
    };
    json_response(id::handle_cleanup(&state, &workspace_root, body.execute))
}

pub async fn id_me_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(id::handle_show(&state, &headers))
}

pub async fn id_lookup_handler(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(id::handle_lookup(&state, params.get("name").cloned()))
}

// ---- browser ----

#[derive(serde::Deserialize)]
pub struct BrowserJoinBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    intro: Option<String>,
    #[serde(default)]
    workspace: Option<String>,
}

pub async fn browser_join_handler(
    State(state): State<AppState>,
    Json(body): Json<BrowserJoinBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(id::handle_browser_join(
        &state,
        body.name,
        body.intro,
        body.workspace,
    ))
}
