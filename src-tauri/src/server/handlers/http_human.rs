use super::human;
use super::json_response;
use crate::proto::ServerMsg;
use crate::server::state::AppState;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;

// ---- human ----

#[derive(serde::Deserialize)]
pub struct HumanInboxQuery {
    #[serde(default)]
    all: bool,
}

pub async fn human_inbox_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HumanInboxQuery>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(human::handle_inbox(&state, &headers, q.all))
}

#[derive(serde::Deserialize)]
pub struct HumanReadBody {
    #[serde(default)]
    message_id: Option<String>,
}

pub async fn human_read_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<HumanReadBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(human::handle_read(&state, &headers, body.message_id))
}

#[derive(serde::Deserialize)]
pub struct HumanReplyBody {
    message_id: String,
    body: String,
    /// 单选兼容字段；多选用 choices。两者并存时合并。
    #[serde(default)]
    choice: Option<String>,
    #[serde(default)]
    choices: Vec<String>,
    #[serde(default = "default_human_surface")]
    surface: String,
    #[serde(default)]
    external_event_id: Option<String>,
}

fn default_human_surface() -> String {
    "api".to_string()
}

pub async fn human_reply_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<HumanReplyBody>,
) -> (StatusCode, Json<ServerMsg>) {
    let mut choices = body.choices;
    if let Some(c) = body.choice {
        choices.push(c);
    }
    json_response(human::handle_reply(
        &state,
        &headers,
        body.message_id,
        body.body,
        choices,
        body.surface,
        body.external_event_id,
    ))
}

#[derive(serde::Deserialize)]
pub struct HumanCancelBody {
    message_id: String,
    #[serde(default = "default_human_surface")]
    surface: String,
    #[serde(default)]
    external_event_id: Option<String>,
}

pub async fn human_cancel_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<HumanCancelBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(human::handle_cancel(
        &state,
        &headers,
        body.message_id,
        body.surface,
        body.external_event_id,
    ))
}

#[derive(serde::Deserialize)]
pub struct HumanDoneBody {
    message_id: String,
    #[serde(default)]
    surface: Option<String>,
    #[serde(default)]
    external_event_id: Option<String>,
}

pub async fn human_done_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<HumanDoneBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(human::handle_done(
        &state,
        &headers,
        body.message_id,
        body.surface,
        body.external_event_id,
    ))
}

pub async fn human_agents_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(human::handle_agents(&state, &headers))
}

#[derive(serde::Deserialize)]
pub struct HumanDeliveryAckBody {
    message_id: String,
    surface: String,
}

pub async fn human_delivery_ack_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<HumanDeliveryAckBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(human::handle_delivery_ack(
        &state,
        &headers,
        body.message_id,
        body.surface,
    ))
}

#[derive(serde::Deserialize)]
pub struct HumanSendBody {
    to: String,
    body: String,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    surface: Option<String>,
    #[serde(default)]
    external_event_id: Option<String>,
}

pub async fn human_send_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<HumanSendBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(human::handle_send(
        &state,
        &headers,
        body.to,
        body.body,
        body.subject,
        body.surface,
        body.external_event_id,
    ))
}
