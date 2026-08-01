use super::json_response;
use super::mem;
use crate::proto::ServerMsg;
use crate::server::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;
use std::collections::HashMap;

// ---- mem ----

pub async fn mem_plan_show_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(mem::handle_plan_show(
        &state,
        &headers,
        params.get("target").cloned(),
    ))
}

#[derive(serde::Deserialize)]
pub struct MemPlanUpdateBody {
    #[serde(default)]
    plan: Option<String>,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    summary: Option<String>,
}

pub async fn mem_plan_update_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MemPlanUpdateBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(mem::handle_plan_update(
        &state,
        &headers,
        body.plan,
        body.context,
        body.status,
        body.summary,
    ))
}

pub async fn mem_plan_status_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(mem::handle_plan_status(
        &state,
        &headers,
        params.get("target").cloned(),
    ))
}

#[derive(serde::Deserialize)]
pub struct MemAddBody {
    text: String,
    topic: String,
    #[serde(rename = "entry_type")]
    ty: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

pub async fn mem_add_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MemAddBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(mem::handle_add(
        &state, &headers, body.text, body.topic, body.ty, body.title, body.tags,
    ))
}

pub async fn mem_search_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> (StatusCode, Json<ServerMsg>) {
    let query = params.get("query").cloned().unwrap_or_default();
    let topic = params.get("topic").cloned();
    let limit = params.get("limit").and_then(|v| v.parse().ok());
    json_response(mem::handle_search(&state, &headers, query, topic, limit))
}

pub async fn mem_show_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(mem::handle_show(&state, &headers, id))
}

pub async fn mem_list_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> (StatusCode, Json<ServerMsg>) {
    let topic = params.get("topic").cloned();
    json_response(mem::handle_list(&state, &headers, topic))
}

pub async fn mem_pack_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> (StatusCode, Json<ServerMsg>) {
    let topic = params.get("topic").cloned();
    let limit = params.get("limit").and_then(|v| v.parse().ok());
    json_response(mem::handle_pack(&state, &headers, topic, limit))
}
