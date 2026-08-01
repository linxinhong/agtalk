use super::json_response;
use super::{config, tool};
use crate::proto::ServerMsg;
use crate::server::state::AppState;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;

// ---- tool ----

pub async fn tool_doctor_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ServerMsg>) {
    let workspace_root = match crate::server::handlers::workspace_root_from_headers(&headers) {
        Ok(root) => root,
        Err(error) => return json_response(error),
    };
    json_response(tool::handle_doctor(&state, &workspace_root))
}

pub async fn tool_version_handler() -> (StatusCode, Json<ServerMsg>) {
    json_response(tool::handle_version())
}

pub async fn tool_path_handler() -> (StatusCode, Json<ServerMsg>) {
    json_response(tool::handle_path())
}

// ---- config ----

pub async fn config_show_handler(State(state): State<AppState>) -> (StatusCode, Json<ServerMsg>) {
    json_response(config::handle_show(&state))
}

pub async fn config_get_handler(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(config::handle_get(&state, key))
}

#[derive(serde::Deserialize)]
pub struct ConfigSetBody {
    value: String,
}

pub async fn config_set_handler(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<ConfigSetBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(config::handle_set(&state, key, body.value))
}

pub async fn config_path_handler(State(state): State<AppState>) -> (StatusCode, Json<ServerMsg>) {
    json_response(config::handle_path(&state))
}
