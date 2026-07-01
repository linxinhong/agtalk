//! HTTP API：/api 处理 ClientMsg，/events 处理 SSE。

use crate::identity::auth::{self, AuthenticatedSession};
use crate::identity::mailbox as mailbox_db;
use crate::identity::{agents_map, session_file};
use crate::proto::{ClientMsg, SendPayload, ServerMsg};
use crate::routing::{inbox, lookup, reply, send, SendRequest};
use crate::server::state::AppState;
use crate::transport::sse::events_stream;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::Sse;
use axum::response::Json;
use axum::routing::{get, post};
use axum::Router;
use std::convert::Infallible;
use sysinfo::{Pid, System};
use tokio_stream::Stream;
use uuid::Uuid;

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/api", post(handle_api))
        .route("/api/send", post(handle_api_send))
        .route("/events", get(events_handler))
        .with_state(state)
}

async fn handle_api(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(msg): Json<ClientMsg>,
) -> (StatusCode, Json<ServerMsg>) {
    let resp = dispatch(state, headers, msg).await;
    let status = match &resp {
        ServerMsg::Error { code, .. } if code == "auth_failed" => StatusCode::UNAUTHORIZED,
        ServerMsg::Error { .. } => StatusCode::BAD_REQUEST,
        _ => StatusCode::OK,
    };
    (status, Json(resp))
}

async fn handle_api_send(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SendPayload>,
) -> (StatusCode, Json<ServerMsg>) {
    let resp = do_send(&state, &headers, payload);
    let status = match &resp {
        ServerMsg::Error { code, .. } if code == "auth_failed" => StatusCode::UNAUTHORIZED,
        ServerMsg::Error { .. } => StatusCode::BAD_REQUEST,
        _ => StatusCode::OK,
    };
    (status, Json(resp))
}

async fn events_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<axum::response::sse::Event, Infallible>>>, StatusCode> {
    let address = headers
        .get("X-AgTalk-Address")
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::BAD_REQUEST)?
        .to_string();
    let pid = headers
        .get("X-AgTalk-Pid")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let start_time = headers
        .get("X-AgTalk-Start-Time")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let last_event_id: Option<i64> = headers
        .get("Last-Event-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());

    auth::authenticate(&state.storage, &state.dot_agtalk, &address, pid, start_time)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let rx = state.registry.subscribe(&address);
    let stream = events_stream(state.storage.clone(), address, last_event_id, rx);
    Ok(Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
}

async fn dispatch(state: AppState, headers: HeaderMap, msg: ClientMsg) -> ServerMsg {
    match msg {
        ClientMsg::Ping => ServerMsg::Pong,
        ClientMsg::Lookup { name } => handle_lookup(&state, name),
        ClientMsg::Send {
            to,
            body,
            content_type,
            reply_to_id,
            metadata,
            more_coming,
        } => do_send(
            &state,
            &headers,
            SendPayload {
                to,
                body,
                content_type,
                reply_to_id,
                metadata,
                more_coming,
            },
        ),
        ClientMsg::Inbox { include_done } => handle_inbox(&state, &headers, include_done),
        ClientMsg::Detail { message_id } => handle_detail(&state, &headers, &message_id),
        ClientMsg::Reply {
            message_id,
            body,
            choice,
        } => handle_reply(&state, &headers, &message_id, &body, choice),
        ClientMsg::Join {
            name,
            intro,
            workspace,
            pid,
            start_time,
        } => handle_join(&state, name, intro, workspace, pid, start_time),
        ClientMsg::Leave { name, .. } => handle_leave(&state, &headers, name),
        ClientMsg::Whoami => handle_whoami(&state, &headers),
        ClientMsg::Human { body, choices } => handle_human(&state, &headers, &body, choices),
    }
}

fn handle_lookup(state: &AppState, name: Option<String>) -> ServerMsg {
    match lookup::lookup(&state.storage, name.as_deref()) {
        Ok(mbs) => ServerMsg::LookupResult { mailboxes: mbs },
        Err(e) => ServerMsg::Error {
            code: "lookup_failed".into(),
            message: e.to_string(),
        },
    }
}

fn do_send(state: &AppState, headers: &HeaderMap, payload: SendPayload) -> ServerMsg {
    let session = match authenticate(state, headers) {
        Ok(s) => s,
        Err(e) => return auth_error(e),
    };
    let to_name = match mailbox_db::get_by_address(&state.storage, &payload.to) {
        Ok(Some(mb)) => mb.name,
        Ok(None) => {
            return ServerMsg::Error {
                code: "send_failed".into(),
                message: format!("目标 mailbox 不存在: {}", payload.to),
            }
        }
        Err(e) => {
            return ServerMsg::Error {
                code: "send_failed".into(),
                message: e.to_string(),
            }
        }
    };
    let req = SendRequest {
        to: &payload.to,
        to_name: &to_name,
        from: &session.address,
        from_name: &session.name,
        body: &payload.body,
        content_type: payload.content_type.as_deref().unwrap_or("text"),
        reply_to_id: payload.reply_to_id.as_deref(),
        metadata: payload.metadata.as_deref().unwrap_or("{}"),
        more_coming: payload.more_coming,
    };
    match send::send(&state.storage, req) {
        Ok(msg) => {
            state.registry.notify(
                &payload.to,
                crate::transport::wake::SseEvent {
                    message: msg.clone(),
                },
            );
            ServerMsg::Ok { id: msg.id }
        }
        Err(e) => ServerMsg::Error {
            code: "send_failed".into(),
            message: e.to_string(),
        },
    }
}

fn handle_inbox(state: &AppState, headers: &HeaderMap, include_done: bool) -> ServerMsg {
    let session = match authenticate(state, headers) {
        Ok(s) => s,
        Err(e) => return auth_error(e),
    };
    match inbox::inbox(&state.storage, &session.address, include_done) {
        Ok(msgs) => ServerMsg::InboxResult { messages: msgs },
        Err(e) => ServerMsg::Error {
            code: "inbox_failed".into(),
            message: e.to_string(),
        },
    }
}

fn handle_reply(
    state: &AppState,
    headers: &HeaderMap,
    message_id: &str,
    body: &str,
    choice: Option<String>,
) -> ServerMsg {
    let session = match authenticate(state, headers) {
        Ok(s) => s,
        Err(e) => return auth_error(e),
    };
    match reply::reply(
        &state.storage,
        message_id,
        &session.address,
        &session.name,
        body,
        choice.as_deref(),
    ) {
        Ok(msg) => {
            state.registry.notify(
                &msg.to_address,
                crate::transport::wake::SseEvent {
                    message: msg.clone(),
                },
            );
            ServerMsg::Ok { id: msg.id }
        }
        Err(e) => ServerMsg::Error {
            code: "reply_failed".into(),
            message: e.to_string(),
        },
    }
}

fn handle_join(
    state: &AppState,
    name: Option<String>,
    intro: Option<String>,
    workspace: Option<String>,
    pid: u32,
    start_time: u64,
) -> ServerMsg {
    let name = name.unwrap_or_else(|| format!("agent-{}", short_id()));
    let intro = intro.unwrap_or_default();
    let workspace = workspace.unwrap_or_default();

    if let Err(e) = validate_pid(pid, start_time) {
        return auth_error(e);
    }

    let address = match mailbox_db::create(&state.storage, &name, &intro, &workspace) {
        Ok(addr) => addr,
        Err(e) => {
            return ServerMsg::Error {
                code: "join_failed".into(),
                message: e.to_string(),
            }
        }
    };

    let session = session_file::SessionFile {
        address: address.clone(),
        name: name.clone(),
        workspace: workspace.clone(),
        intro: intro.clone(),
        created_at: iso_now(),
    };

    if let Err(e) = session_file::write(&state.dot_agtalk, &name, &session) {
        return ServerMsg::Error {
            code: "session_write_failed".into(),
            message: e.to_string(),
        };
    }
    if let Err(e) = agents_map::register_pid(&state.dot_agtalk, pid, &name, start_time) {
        return ServerMsg::Error {
            code: "agents_map_failed".into(),
            message: e.to_string(),
        };
    }

    ServerMsg::Ok { id: address }
}

fn handle_detail(state: &AppState, headers: &HeaderMap, message_id: &str) -> ServerMsg {
    let session = match authenticate(state, headers) {
        Ok(s) => s,
        Err(e) => return auth_error(e),
    };
    match lookup::detail_and_mark_read(&state.storage, &session.address, message_id) {
        Ok(Some(msg)) => ServerMsg::MessageDetail(msg),
        Ok(None) => ServerMsg::Error {
            code: "not_found".into(),
            message: "消息不存在".into(),
        },
        Err(e) => ServerMsg::Error {
            code: "detail_failed".into(),
            message: e.to_string(),
        },
    }
}

fn handle_leave(state: &AppState, headers: &HeaderMap, name: Option<String>) -> ServerMsg {
    let session = match authenticate(state, headers) {
        Ok(s) => s,
        Err(e) => return auth_error(e),
    };

    let name = name.unwrap_or(session.name);

    if let Err(e) = mailbox_db::mark_left(&state.storage, &session.address) {
        return ServerMsg::Error {
            code: "leave_failed".into(),
            message: e.to_string(),
        };
    }
    if let Err(e) = session_file::remove(&state.dot_agtalk, &name) {
        return ServerMsg::Error {
            code: "leave_failed".into(),
            message: e.to_string(),
        };
    }
    if let Some(pid) = session.pid {
        let _ = agents_map::remove_pid(&state.dot_agtalk, pid);
    }

    ServerMsg::Pong
}

fn handle_whoami(state: &AppState, headers: &HeaderMap) -> ServerMsg {
    let session = match authenticate(state, headers) {
        Ok(s) => s,
        Err(e) => return auth_error(e),
    };
    ServerMsg::WhoamiResult {
        address: session.address,
        name: session.name,
        workspace: session.workspace,
        intro: "".into(),
    }
}

fn handle_human(
    state: &AppState,
    headers: &HeaderMap,
    body: &str,
    choices: Option<Vec<String>>,
) -> ServerMsg {
    let session = match authenticate(state, headers) {
        Ok(s) => s,
        Err(e) => return auth_error(e),
    };

    let human_address = match mailbox_db::ensure_human(&state.storage, &state.config.human) {
        Ok(addr) => addr,
        Err(e) => {
            return ServerMsg::Error {
                code: "human_failed".into(),
                message: e.to_string(),
            }
        }
    };

    let content_type = if choices.is_some() {
        "approval_request"
    } else {
        "text"
    };
    let metadata = choices
        .map(|c| serde_json::json!({ "choices": c }).to_string())
        .unwrap_or_else(|| "{}".to_string());

    let req = SendRequest {
        to: &human_address,
        to_name: &state.config.human.name,
        from: &session.address,
        from_name: &session.name,
        body,
        content_type,
        reply_to_id: None,
        metadata: &metadata,
        more_coming: false,
    };
    match send::send(&state.storage, req) {
        Ok(msg) => {
            state.registry.notify(
                &human_address,
                crate::transport::wake::SseEvent {
                    message: msg.clone(),
                },
            );
            ServerMsg::Ok { id: msg.id }
        }
        Err(e) => ServerMsg::Error {
            code: "human_failed".into(),
            message: e.to_string(),
        },
    }
}

fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<AuthenticatedSession, String> {
    let address = headers
        .get("X-AgTalk-Address")
        .and_then(|v| v.to_str().ok())
        .ok_or("缺少 X-AgTalk-Address")?;
    let pid = headers
        .get("X-AgTalk-Pid")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let start_time = headers
        .get("X-AgTalk-Start-Time")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());

    auth::authenticate(&state.storage, &state.dot_agtalk, address, pid, start_time)
        .map_err(|e| e.to_string())
}

fn auth_error(message: String) -> ServerMsg {
    ServerMsg::Error {
        code: "auth_failed".into(),
        message,
    }
}

fn validate_pid(pid: u32, start_time: u64) -> Result<(), String> {
    let mut sys = System::new_all();
    sys.refresh_processes();
    let process = sys.process(Pid::from(pid as usize)).ok_or("进程不存在")?;
    if process.start_time() != start_time {
        return Err("PID start_time 不匹配".into());
    }
    Ok(())
}

fn short_id() -> String {
    Uuid::new_v4()
        .to_string()
        .split('-')
        .next()
        .unwrap_or("")
        .to_string()
}

fn iso_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgConfig;
    use crate::identity::{mailbox, session_file};
    use crate::proto::ServerMsg;
    use crate::storage::Storage;
    use axum::body::Body;
    use axum::http::Request;
    use tempfile::TempDir;
    use tower::ServiceExt;

    fn test_state() -> (AppState, String, String, TempDir) {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        let storage = Storage::open_in_memory().unwrap();
        let nora = mailbox::create(&storage, "nora", "前端", "projA").unwrap();
        let quinn = mailbox::create(&storage, "quinn", "后端", "projB").unwrap();

        session_file::write(
            &dot,
            "nora",
            &session_file::SessionFile {
                address: nora.clone(),
                name: "nora".to_string(),
                workspace: "projA".to_string(),
                intro: "前端".to_string(),
                created_at: "2026-07-01T00:00:00Z".to_string(),
            },
        )
        .unwrap();

        let state = AppState::new(storage, AgConfig::default(), dot);
        (state, nora, quinn, tmp)
    }

    #[tokio::test]
    async fn http_post_send_ok() {
        let (state, nora, quinn, _tmp) = test_state();
        let app = routes(state.clone());

        let body = serde_json::to_string(&serde_json::json!({
            "to": quinn,
            "body": "hi from http",
        }))
        .unwrap();

        let request = Request::builder()
            .method("POST")
            .uri("/api/send")
            .header("Content-Type", "application/json")
            .header("X-AgTalk-Address", nora.clone())
            .body(Body::from(body))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        let msg_id = match resp {
            ServerMsg::Ok { id } => id,
            other => panic!("expected Ok, got {:?}", other),
        };

        let conn = state.storage.conn();
        let msg: crate::routing::Message = conn
            .query_row(
                "SELECT * FROM messages WHERE id = ?1",
                [&msg_id],
                crate::routing::Message::from_row,
            )
            .unwrap();
        assert_eq!(msg.to_address, quinn);
        assert_eq!(msg.to_name, "quinn");
        assert_eq!(msg.from_address, nora);
        assert_eq!(msg.from_name, "nora");
        assert_eq!(msg.body, "hi from http");
    }

    #[tokio::test]
    async fn http_post_send_missing_auth() {
        let (state, _nora, quinn, _tmp) = test_state();
        let app = routes(state);

        let body = serde_json::to_string(&serde_json::json!({
            "to": quinn,
            "body": "hi",
        }))
        .unwrap();

        let request = Request::builder()
            .method("POST")
            .uri("/api/send")
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
