//! HTTP API v1：canonical `/api/v1/*` 路由。

use crate::proto::ServerMsg;
use crate::server::handlers::{config, daemon, id, mem, msg, run, status_for, tool};
use crate::server::state::AppState;
use crate::transport::sse::events_stream;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::Sse;
use axum::response::Json;
use axum::routing::{get, patch, post};
use axum::Router;
use std::collections::HashMap;
use std::convert::Infallible;
use tokio_stream::Stream;

pub fn routes(state: AppState) -> Router {
    Router::new()
        // id
        .route("/api/v1/id/join", post(id_join_handler))
        .route("/api/v1/id/leave", post(id_leave_handler))
        .route("/api/v1/id/me", get(id_me_handler))
        .route("/api/v1/id/lookup", get(id_lookup_handler))
        // msg
        .route("/api/v1/msg/send", post(msg_send_handler))
        .route("/api/v1/msg/reply", post(msg_reply_handler))
        .route("/api/v1/msg/done", post(msg_done_handler))
        .route("/api/v1/msg/ask", post(msg_ask_handler))
        .route("/api/v1/msg/inbox", get(msg_inbox_handler))
        .route("/api/v1/msg/read", post(msg_read_handler))
        .route("/api/v1/msg/wait", post(msg_wait_handler))
        .route("/api/v1/msg/attachment/:id", get(msg_attachment_handler))
        // mem
        .route("/api/v1/mem/plan", get(mem_plan_show_handler))
        .route("/api/v1/mem/plan", patch(mem_plan_update_handler))
        .route("/api/v1/mem/plan/status", get(mem_plan_status_handler))
        .route("/api/v1/mem/add", post(mem_add_handler))
        .route("/api/v1/mem/search", get(mem_search_handler))
        .route("/api/v1/mem/show/:id", get(mem_show_handler))
        .route("/api/v1/mem/list", get(mem_list_handler))
        .route("/api/v1/mem/pack", get(mem_pack_handler))
        // tool
        .route("/api/v1/tool/daemon", post(tool_daemon_handler))
        .route("/api/v1/tool/doctor", post(tool_doctor_handler))
        .route("/api/v1/tool/version", get(tool_version_handler))
        .route("/api/v1/tool/path", get(tool_path_handler))
        // config
        .route("/api/v1/config", get(config_show_handler))
        .route("/api/v1/config/:key", get(config_get_handler))
        .route("/api/v1/config/:key", patch(config_set_handler))
        .route("/api/v1/config/path", get(config_path_handler))
        // run
        .route("/api/v1/run", post(run_handler))
        // daemon
        .route("/api/v1/daemon/status", get(daemon::daemon_status_handler))
        // browser
        .route("/api/v1/browser/join", post(browser_join_handler))
        // events
        .route("/api/v1/events", get(events_handler))
        .with_state(state)
}

fn json_response(resp: ServerMsg) -> (StatusCode, Json<ServerMsg>) {
    let status = status_for(&resp);
    (status, Json(resp))
}

// ---- id ----

#[derive(serde::Deserialize)]
struct IdJoinBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    intro: Option<String>,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default = "crate::proto::default_notify")]
    notify: String,
    pid: u32,
    start_time: u64,
}

async fn id_join_handler(
    State(state): State<AppState>,
    Json(body): Json<IdJoinBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(id::handle_join(
        &state,
        body.name,
        body.intro,
        body.workspace,
        body.notify,
        body.pid,
        body.start_time,
    ))
}

#[derive(serde::Deserialize)]
struct IdLeaveBody {
    #[serde(default)]
    purge: bool,
}

async fn id_leave_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<IdLeaveBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(id::handle_leave(&state, &headers, body.purge))
}

async fn id_me_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(id::handle_show(&state, &headers))
}

async fn id_lookup_handler(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(id::handle_lookup(&state, params.get("name").cloned()))
}

// ---- msg ----

#[derive(serde::Deserialize)]
struct MsgSendBody {
    to: String,
    body: String,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    notify: Option<bool>,
    #[serde(default)]
    more: bool,
}

async fn msg_send_handler(
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
        body.more,
    ))
}

#[derive(serde::Deserialize)]
struct MsgReplyBody {
    message_id: String,
    body: String,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    notify: Option<bool>,
}

async fn msg_reply_handler(
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
    ))
}

#[derive(serde::Deserialize)]
struct MsgDoneBody {
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    files: Vec<String>,
}

async fn msg_done_handler(
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
struct MsgAskBody {
    message: String,
    #[serde(default)]
    questions: Vec<String>,
    #[serde(default)]
    options: crate::proto::AskOptions,
    #[serde(default)]
    wait: bool,
    #[serde(default)]
    timeout: Option<u64>,
}

async fn msg_ask_handler(
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
    ))
}

async fn msg_inbox_handler(
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
struct MsgReadBody {
    #[serde(default)]
    message_id: Option<String>,
}

async fn msg_read_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MsgReadBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(msg::handle_read(&state, &headers, body.message_id))
}

#[derive(serde::Deserialize)]
struct MsgWaitBody {
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    timeout: Option<u64>,
    #[serde(default)]
    since: Option<i64>,
}

async fn msg_wait_handler(
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

async fn msg_attachment_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(msg::handle_attachment(&state, &headers, id))
}

// ---- mem ----

async fn mem_plan_show_handler(
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
struct MemPlanUpdateBody {
    #[serde(default)]
    plan: Option<String>,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    summary: Option<String>,
}

async fn mem_plan_update_handler(
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

async fn mem_plan_status_handler(
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
struct MemAddBody {
    text: String,
    topic: String,
    #[serde(rename = "entry_type")]
    ty: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

async fn mem_add_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MemAddBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(mem::handle_add(
        &state, &headers, body.text, body.topic, body.ty, body.title, body.tags,
    ))
}

async fn mem_search_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> (StatusCode, Json<ServerMsg>) {
    let query = params.get("query").cloned().unwrap_or_default();
    let topic = params.get("topic").cloned();
    let limit = params.get("limit").and_then(|v| v.parse().ok());
    json_response(mem::handle_search(&state, &headers, query, topic, limit))
}

async fn mem_show_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(mem::handle_show(&state, &headers, id))
}

async fn mem_list_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> (StatusCode, Json<ServerMsg>) {
    let topic = params.get("topic").cloned();
    json_response(mem::handle_list(&state, &headers, topic))
}

async fn mem_pack_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> (StatusCode, Json<ServerMsg>) {
    let topic = params.get("topic").cloned();
    let limit = params.get("limit").and_then(|v| v.parse().ok());
    json_response(mem::handle_pack(&state, &headers, topic, limit))
}

// ---- tool ----

#[derive(serde::Deserialize)]
struct ToolDaemonBody {
    #[serde(default)]
    action: Option<String>,
}

async fn tool_daemon_handler(
    State(state): State<AppState>,
    Json(body): Json<ToolDaemonBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(tool::handle_daemon(&state, body.action))
}

async fn tool_doctor_handler(State(state): State<AppState>) -> (StatusCode, Json<ServerMsg>) {
    json_response(tool::handle_doctor(&state))
}

async fn tool_version_handler() -> (StatusCode, Json<ServerMsg>) {
    json_response(tool::handle_version())
}

async fn tool_path_handler() -> (StatusCode, Json<ServerMsg>) {
    json_response(tool::handle_path())
}

// ---- config ----

async fn config_show_handler(State(state): State<AppState>) -> (StatusCode, Json<ServerMsg>) {
    json_response(config::handle_show(&state))
}

async fn config_get_handler(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(config::handle_get(&state, key))
}

#[derive(serde::Deserialize)]
struct ConfigSetBody {
    value: String,
}

async fn config_set_handler(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<ConfigSetBody>,
) -> (StatusCode, Json<ServerMsg>) {
    json_response(config::handle_set(&state, key, body.value))
}

async fn config_path_handler(State(state): State<AppState>) -> (StatusCode, Json<ServerMsg>) {
    json_response(config::handle_path(&state))
}

// ---- run ----

#[derive(serde::Deserialize)]
struct RunBody {
    #[serde(default)]
    file: Option<String>,
}

async fn run_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RunBody>,
) -> (StatusCode, Json<ServerMsg>) {
    let ctx = auth_context_from_headers(&headers);
    json_response(run::handle_run(&state, body.file, ctx))
}

fn auth_context_from_headers(headers: &HeaderMap) -> Option<crate::run::AuthContext> {
    let address = headers
        .get("X-AgTalk-Address")
        .and_then(|v| v.to_str().ok())?;
    let pid = headers
        .get("X-AgTalk-Pid")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())?;
    let start_time = headers
        .get("X-AgTalk-Start-Time")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())?;
    Some(crate::run::AuthContext {
        address: address.to_string(),
        pid,
        start_time,
    })
}

// ---- browser ----

#[derive(serde::Deserialize)]
struct BrowserJoinBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    intro: Option<String>,
    #[serde(default)]
    workspace: Option<String>,
}

async fn browser_join_handler(
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

// ---- events ----

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
    let browser_token = headers
        .get("X-AgTalk-Browser-Token")
        .and_then(|v| v.to_str().ok());

    crate::identity::auth::authenticate(
        &state.storage,
        &state.dot_agtalk,
        &address,
        pid,
        start_time,
        browser_token,
    )
    .map_err(|_| StatusCode::UNAUTHORIZED)?;

    let rx = state.registry.subscribe(&address);
    let stream = events_stream(state.storage.clone(), address, last_event_id, rx);
    Ok(Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgConfig;
    use crate::identity::browser_session;
    use crate::identity::{mailbox, mailbox as mailbox_db, session_file};
    use crate::proto::ServerMsg;
    use crate::storage::Storage;
    use axum::body::Body;
    use axum::http::Request;
    use std::ffi::OsString;
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
                ..Default::default()
            },
        )
        .unwrap();

        crate::mem::index::register(
            &storage,
            &nora,
            "nora",
            "projA",
            &dot.join("nora").join("memory"),
        );

        let state = AppState::new(storage, AgConfig::default(), dot);
        (state, nora, quinn, tmp)
    }

    struct EnvGuard(Option<OsString>);

    impl EnvGuard {
        fn set(path: &std::path::Path) -> Self {
            let previous = std::env::var_os("AGTALK_CONFIG_DIR");
            std::env::set_var("AGTALK_CONFIG_DIR", path);
            Self(previous)
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(ref p) = self.0 {
                std::env::set_var("AGTALK_CONFIG_DIR", p);
            } else {
                std::env::remove_var("AGTALK_CONFIG_DIR");
            }
        }
    }

    fn browser_test_state() -> (AppState, TempDir, TempDir, EnvGuard) {
        let tmp = TempDir::new().unwrap();
        let browser_tmp = TempDir::new().unwrap();
        let guard = EnvGuard::set(browser_tmp.path());
        let storage = Storage::open_in_memory().unwrap();
        let state = AppState::new(storage, AgConfig::default(), tmp.path().join(".agtalk"));
        (state, tmp, browser_tmp, guard)
    }

    #[tokio::test]
    async fn v1_daemon_status_returns_running() {
        let (state, _nora, _quinn, _tmp) = test_state();
        crate::server::daemon::write_status_file(
            std::process::id(),
            crate::server::daemon::now_unix_secs(),
            19527,
            env!("CARGO_PKG_VERSION"),
            std::path::Path::new("/tmp/config.json"),
            std::path::Path::new("/tmp/agtalk.db"),
        )
        .unwrap();

        let app = routes(state.clone());
        let request = Request::builder()
            .method("GET")
            .uri("/api/v1/daemon/status")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        match resp {
            ServerMsg::DaemonStatus {
                active_mailboxes,
                pending_messages,
                ..
            } => {
                assert_eq!(active_mailboxes, 2);
                assert_eq!(pending_messages, 0);
            }
            other => panic!("expected DaemonStatus, got {:?}", other),
        }

        crate::server::daemon::remove_status_file();
    }

    #[tokio::test]
    async fn v1_msg_send_ok() {
        let (state, nora, quinn, _tmp) = test_state();
        let app = routes(state.clone());

        let body = serde_json::to_string(&serde_json::json!({
            "to": quinn,
            "body": "hi from http",
        }))
        .unwrap();

        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/msg/send")
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
    async fn v1_msg_send_missing_auth() {
        let (state, _nora, quinn, _tmp) = test_state();
        let app = routes(state);

        let body = serde_json::to_string(&serde_json::json!({
            "to": quinn,
            "body": "hi",
        }))
        .unwrap();

        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/msg/send")
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn v1_browser_join_lookup_leave() {
        let (state, _tmp, _browser_tmp, _guard) = browser_test_state();
        let app = routes(state.clone());

        let body = serde_json::to_string(&serde_json::json!({
            "name": "browser-agent",
            "intro": "bridge",
            "workspace": "web"
        }))
        .unwrap();
        let join_req = Request::builder()
            .method("POST")
            .uri("/api/v1/browser/join")
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let join_resp = app.clone().oneshot(join_req).await.unwrap();
        assert_eq!(join_resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(join_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let join: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        let (addr, token) = match join {
            ServerMsg::BrowserJoinResult { address, token, .. } => (address, token),
            other => panic!("expected BrowserJoinResult, got {:?}", other),
        };

        let lookup_req = Request::builder()
            .method("GET")
            .uri("/api/v1/id/lookup?name=browser-agent")
            .body(Body::empty())
            .unwrap();
        let lookup_resp = app.clone().oneshot(lookup_req).await.unwrap();
        assert_eq!(lookup_resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(lookup_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let lookup: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        let found = match lookup {
            ServerMsg::LookupResult { mailboxes } => mailboxes
                .iter()
                .any(|m| m.address == addr && m.name == "browser-agent"),
            other => panic!("expected LookupResult, got {:?}", other),
        };
        assert!(found);

        let leave_req = Request::builder()
            .method("POST")
            .uri("/api/v1/id/leave")
            .header("Content-Type", "application/json")
            .header("X-AgTalk-Address", addr.clone())
            .header("X-AgTalk-Browser-Token", token)
            .body(Body::from(r#"{}"#))
            .unwrap();
        let leave_resp = app.clone().oneshot(leave_req).await.unwrap();
        assert_eq!(leave_resp.status(), StatusCode::OK);

        let lookup_req2 = Request::builder()
            .method("GET")
            .uri("/api/v1/id/lookup?name=browser-agent")
            .body(Body::empty())
            .unwrap();
        let lookup_resp2 = app.clone().oneshot(lookup_req2).await.unwrap();
        let bytes = axum::body::to_bytes(lookup_resp2.into_body(), usize::MAX)
            .await
            .unwrap();
        let lookup2: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        match lookup2 {
            ServerMsg::LookupResult { mailboxes } => assert!(mailboxes.is_empty()),
            other => panic!("expected LookupResult, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn v1_browser_send_via_token() {
        let (state, _tmp, _browser_tmp, _guard) = browser_test_state();
        let app = routes(state.clone());

        let (addr, _name, token) = browser_session::create(
            &state.storage,
            Some("browser-agent".to_string()),
            Some("bridge".to_string()),
            Some("web".to_string()),
        )
        .unwrap();

        let recipient = mailbox::create(&state.storage, "quinn", "后端", "projB").unwrap();

        let body = serde_json::to_string(&serde_json::json!({
            "to": recipient,
            "body": "hello from browser",
        }))
        .unwrap();
        let send_req = Request::builder()
            .method("POST")
            .uri("/api/v1/msg/send")
            .header("Content-Type", "application/json")
            .header("X-AgTalk-Address", addr.clone())
            .header("X-AgTalk-Browser-Token", token)
            .body(Body::from(body))
            .unwrap();
        let send_resp = app.oneshot(send_req).await.unwrap();
        assert_eq!(send_resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(send_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let send: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        let msg_id = match send {
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
        assert_eq!(msg.to_address, recipient);
        assert_eq!(msg.from_address, addr);
        assert_eq!(msg.from_name, "browser-agent");
        assert_eq!(msg.body, "hello from browser");
    }

    #[tokio::test]
    async fn v1_id_join_idempotent_reuses_address() {
        let (state, nora, _quinn, _tmp) = test_state();
        let app = routes(state.clone());

        let cur_pid = std::process::id();
        let mut sys = sysinfo::System::new_all();
        sys.refresh_processes();
        let cur_start = sys
            .process(sysinfo::Pid::from(cur_pid as usize))
            .map(|p| p.start_time())
            .unwrap_or(1);

        let body1 = serde_json::to_string(&serde_json::json!({
            "name": "nora",
            "intro": "前端",
            "workspace": "projA",
            "notify": "none",
            "pid": cur_pid,
            "start_time": cur_start,
        }))
        .unwrap();
        let join1 = Request::builder()
            .method("POST")
            .uri("/api/v1/id/join")
            .header("Content-Type", "application/json")
            .body(Body::from(body1))
            .unwrap();
        let resp1 = app.clone().oneshot(join1).await.unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);

        let addr1 = match serde_json::from_slice::<ServerMsg>(
            &axum::body::to_bytes(resp1.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap()
        {
            ServerMsg::Ok { id } => id,
            other => panic!("expected Ok, got {:?}", other),
        };

        let body2 = serde_json::to_string(&serde_json::json!({
            "name": "nora",
            "intro": "后端",
            "workspace": "projB",
            "notify": "none",
            "pid": cur_pid,
            "start_time": cur_start,
        }))
        .unwrap();
        let join2 = Request::builder()
            .method("POST")
            .uri("/api/v1/id/join")
            .header("Content-Type", "application/json")
            .body(Body::from(body2))
            .unwrap();
        let resp2 = app.clone().oneshot(join2).await.unwrap();
        let addr2 = match serde_json::from_slice::<ServerMsg>(
            &axum::body::to_bytes(resp2.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap()
        {
            ServerMsg::Ok { id } => id,
            other => panic!("expected Ok, got {:?}", other),
        };

        assert_eq!(addr1, addr2);
        assert_eq!(addr1, nora);

        let mb = mailbox_db::get_by_address(&state.storage, &addr1)
            .unwrap()
            .unwrap();
        assert_eq!(mb.intro, "后端");
        assert_eq!(mb.workspace, "projB");
    }

    #[tokio::test]
    async fn v1_id_join_revives_left_mailbox() {
        let (state, nora, _quinn, _tmp) = test_state();
        mailbox_db::mark_left(&state.storage, &nora).unwrap();
        assert!(mailbox_db::get_by_address(&state.storage, &nora)
            .unwrap()
            .is_none());

        let cur_pid = std::process::id();
        let mut sys = sysinfo::System::new_all();
        sys.refresh_processes();
        let cur_start = sys
            .process(sysinfo::Pid::from(cur_pid as usize))
            .map(|p| p.start_time())
            .unwrap_or(1);

        let app = routes(state.clone());
        let body = serde_json::to_string(&serde_json::json!({
            "name": "nora",
            "notify": "none",
            "pid": cur_pid,
            "start_time": cur_start,
        }))
        .unwrap();
        let join = Request::builder()
            .method("POST")
            .uri("/api/v1/id/join")
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(join).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let mb = mailbox_db::get_by_address(&state.storage, &nora)
            .unwrap()
            .unwrap();
        assert_eq!(mb.address, nora);
        assert!(mb.left_at.is_none());
    }

    #[tokio::test]
    async fn v1_msg_read_dash_empty_inbox() {
        let (state, nora, _quinn, _tmp) = test_state();
        let app = routes(state.clone());

        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/msg/read")
            .header("Content-Type", "application/json")
            .header("X-AgTalk-Address", nora.clone())
            .body(Body::from(r#"{"message_id":"-"}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let msg: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        match msg {
            ServerMsg::Error { code, .. } => assert_eq!(code, "not_found"),
            other => panic!("expected Error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn v1_mem_plan_update_and_show() {
        let (state, nora, _quinn, _tmp) = test_state();
        let app = routes(state.clone());

        let update = serde_json::to_string(&serde_json::json!({
            "plan": "do X",
            "context": "ctx",
            "status": "running",
            "summary": "50%",
        }))
        .unwrap();
        let update_req = Request::builder()
            .method("PATCH")
            .uri("/api/v1/mem/plan")
            .header("Content-Type", "application/json")
            .header("X-AgTalk-Address", nora.clone())
            .body(Body::from(update))
            .unwrap();
        let update_resp = app.clone().oneshot(update_req).await.unwrap();
        assert_eq!(update_resp.status(), StatusCode::OK);

        let show_req = Request::builder()
            .method("GET")
            .uri("/api/v1/mem/plan")
            .header("X-AgTalk-Address", nora.clone())
            .body(Body::empty())
            .unwrap();
        let show_resp = app.clone().oneshot(show_req).await.unwrap();
        assert_eq!(show_resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(show_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let show: ServerMsg = serde_json::from_slice(&bytes).unwrap();
        match show {
            ServerMsg::MemPlanShow {
                address,
                plan,
                context,
                status,
                summary,
                ..
            } => {
                assert_eq!(address, nora);
                assert_eq!(plan, "do X");
                assert_eq!(context, "ctx");
                assert_eq!(status, "running");
                assert_eq!(summary, "50%");
            }
            other => panic!("expected MemPlanShow, got {:?}", other),
        }

        // mem_index 应被刷新
        let row = crate::mem::index::lookup_by_address(&state.storage, &nora).unwrap();
        assert_eq!(row.status_summary, "running: 50%");
        assert!(row.plan_updated_at > 0.0);
    }
}
