use super::json_response;
use super::msg;
use crate::proto::ServerMsg;
use crate::server::state::AppState;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;
use std::collections::HashMap;

// ---- msg ----

#[derive(serde::Deserialize)]
pub struct MsgSendBody {
    to: String,
    body: String,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    notify: Option<bool>,
    #[serde(default)]
    send_enter: Option<bool>,
    #[serde(default)]
    more: bool,
}

pub async fn msg_send_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MsgSendBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(msg::handle_send(
        &state,
        &headers,
        body.to,
        body.body,
        body.subject,
        body.files,
        body.notify,
        body.send_enter,
        body.more,
    ))
}

#[derive(serde::Deserialize)]
pub struct MsgReplyBody {
    message_id: String,
    body: String,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    notify: Option<bool>,
    #[serde(default)]
    send_enter: Option<bool>,
}

pub async fn msg_reply_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MsgReplyBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(msg::handle_reply(
        &state,
        &headers,
        body.message_id,
        body.body,
        body.files,
        body.notify,
        body.send_enter,
    ))
}

#[derive(serde::Deserialize)]
pub struct MsgDoneBody {
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    files: Vec<String>,
}

pub async fn msg_done_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MsgDoneBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(msg::handle_done(
        &state,
        &headers,
        body.message_id,
        body.body,
        body.files,
    ))
}

#[derive(serde::Deserialize)]
pub struct MsgAskBody {
    message: String,
    #[serde(default)]
    questions: Vec<String>,
    #[serde(default)]
    options: crate::proto::AskOptions,
    #[serde(default)]
    wait: bool,
    #[serde(default)]
    timeout: Option<u64>,
    #[serde(default)]
    notify: Option<bool>,
}

pub async fn msg_ask_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MsgAskBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(msg::handle_ask(
        &state,
        &headers,
        body.message,
        body.questions,
        body.options,
        body.wait,
        body.timeout,
        body.notify.unwrap_or(true),
    ))
}

pub async fn msg_inbox_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> (StatusCode, Json<ServerMsg>) {
    let all = params
        .get("all")
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let filter = crate::proto::InboxFilter {
        all,
        ..Default::default()
    };
    json_response(msg::handle_inbox(&state, &headers, filter))
}

#[derive(serde::Deserialize)]
pub struct MsgReadBody {
    #[serde(default)]
    message_id: Option<String>,
}

pub async fn msg_read_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MsgReadBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(msg::handle_read(&state, &headers, body.message_id))
}

#[derive(serde::Deserialize)]
pub struct MsgWaitBody {
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    timeout: Option<u64>,
    #[serde(default)]
    since: Option<i64>,
}

pub async fn msg_wait_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MsgWaitBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(msg::handle_wait(
        &state,
        &headers,
        body.message_id,
        body.timeout,
        body.since,
    ))
}
