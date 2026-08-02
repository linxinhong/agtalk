//! HTTP API v1：canonical `/api/v1/*` 路由。

use crate::server::handlers::{daemon, graph, http_human, http_id, http_mem, http_msg, http_tool};
use crate::server::state::AppState;
use crate::transport::sse::events_stream;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::Sse;
use axum::routing::{delete, get, patch, post};
use axum::Router;
use std::convert::Infallible;
use tokio_stream::Stream;

pub fn routes(state: AppState) -> Router {
    Router::new()
        // id
        .route("/api/v1/id/join", post(http_id::id_join_handler))
        .route("/api/v1/id/leave", post(http_id::id_leave_handler))
        .route("/api/v1/id/cleanup", post(http_id::id_cleanup_handler))
        .route("/api/v1/id/me", get(http_id::id_me_handler))
        .route("/api/v1/id/lookup", get(http_id::id_lookup_handler))
        // msg
        .route("/api/v1/msg/send", post(http_msg::msg_send_handler))
        .route("/api/v1/msg/reply", post(http_msg::msg_reply_handler))
        .route("/api/v1/msg/done", post(http_msg::msg_done_handler))
        .route("/api/v1/msg/ask", post(http_msg::msg_ask_handler))
        .route("/api/v1/msg/inbox", get(http_msg::msg_inbox_handler))
        .route("/api/v1/msg/read", post(http_msg::msg_read_handler))
        .route("/api/v1/msg/wait", post(http_msg::msg_wait_handler))
        // mem
        .route("/api/v1/mem/plan", get(http_mem::mem_plan_show_handler))
        .route("/api/v1/mem/plan", patch(http_mem::mem_plan_update_handler))
        .route(
            "/api/v1/mem/plan/status",
            get(http_mem::mem_plan_status_handler),
        )
        .route("/api/v1/mem/add", post(http_mem::mem_add_handler))
        .route("/api/v1/mem/search", get(http_mem::mem_search_handler))
        .route("/api/v1/mem/show/:id", get(http_mem::mem_show_handler))
        .route("/api/v1/mem/list", get(http_mem::mem_list_handler))
        .route("/api/v1/mem/pack", get(http_mem::mem_pack_handler))
        // tool
        .route("/api/v1/tool/doctor", post(http_tool::tool_doctor_handler))
        .route("/api/v1/tool/version", get(http_tool::tool_version_handler))
        .route("/api/v1/tool/path", get(http_tool::tool_path_handler))
        // config
        .route("/api/v1/config", get(http_tool::config_show_handler))
        .route("/api/v1/config/:key", get(http_tool::config_get_handler))
        .route("/api/v1/config/:key", patch(http_tool::config_set_handler))
        .route("/api/v1/config/path", get(http_tool::config_path_handler))
        // daemon
        .route("/api/v1/daemon/status", get(daemon::daemon_status_handler))
        // browser
        .route("/api/v1/browser/join", post(http_id::browser_join_handler))
        // human（仅 X-AgTalk-Human-Token 可访问）
        .route("/api/v1/human/inbox", get(http_human::human_inbox_handler))
        .route("/api/v1/human/read", post(http_human::human_read_handler))
        .route("/api/v1/human/reply", post(http_human::human_reply_handler))
        .route("/api/v1/human/done", post(http_human::human_done_handler))
        .route(
            "/api/v1/human/cancel",
            post(http_human::human_cancel_handler),
        )
        .route(
            "/api/v1/human/delivery/ack",
            post(http_human::human_delivery_ack_handler),
        )
        .route(
            "/api/v1/human/agents",
            get(http_human::human_agents_handler),
        )
        .route("/api/v1/human/send", post(http_human::human_send_handler))
        // graph（图工程，docs/design_graph.md §6）
        .route("/api/v1/graph/submit", post(graph::graph_submit_handler))
        .route("/api/v1/graph/runs", get(graph::graph_runs_handler))
        .route("/api/v1/graph/runs/:id", get(graph::graph_run_show_handler))
        .route(
            "/api/v1/graph/runs/:id/events",
            get(graph::graph_run_events_handler),
        )
        .route(
            "/api/v1/graph/runs/:id/control",
            post(graph::graph_run_control_handler),
        )
        .route(
            "/api/v1/graph/runs/:id/patch",
            post(graph::graph_run_patch_handler),
        )
        .route(
            "/api/v1/graph/runs/:id",
            delete(graph::graph_run_delete_handler),
        )
        .route(
            "/api/v1/graph/events/stream",
            get(graph_events_stream_handler),
        )
        .route(
            "/api/v1/graph/node/heartbeat",
            post(graph::graph_node_heartbeat_handler),
        )
        .route(
            "/api/v1/graph/node/result",
            post(graph::graph_node_result_handler),
        )
        // events
        .route("/api/v1/events", get(events_handler))
        .with_state(state)
}

// ---- graph events (SSE) ----

#[derive(serde::Deserialize)]
struct GraphEventsStreamQuery {
    run_id: String,
}

/// GraphEvent SSE 订阅（docs/design_graph.md §7）：按 run_id，Last-Event-ID 断线重放。
async fn graph_events_stream_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<GraphEventsStreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<axum::response::sse::Event, Infallible>>>, StatusCode> {
    // 读取端点认证：human token（图管理界面）或 agent
    graph::authenticate_read(&state, &headers).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let last_event_id: Option<i64> = headers
        .get("Last-Event-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let rx = state.graph_events.subscribe(&q.run_id);
    let stream = crate::transport::graph_hub::graph_events_stream(
        state.storage.clone(),
        q.run_id,
        last_event_id,
        rx,
    );
    Ok(Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
}

// ---- events ----

async fn events_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<axum::response::sse::Event, Infallible>>>, StatusCode> {
    let last_event_id: Option<i64> = headers
        .get("Last-Event-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());

    // human 客户端分支：X-AgTalk-Human-Token 订阅 human mailbox 的统一 SSE（无需 X-AgTalk-Address）
    if let Some(human_token) = headers
        .get("X-AgTalk-Human-Token")
        .and_then(|v| v.to_str().ok())
    {
        let session = crate::identity::human_session::validate(human_token)
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
        let human_addr = crate::human::human_address(&state.storage)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if session.address != human_addr {
            return Err(StatusCode::UNAUTHORIZED);
        }
        let rx = state.registry.subscribe(&human_addr);
        let stream = events_stream(state.storage.clone(), human_addr, last_event_id, rx);
        return Ok(Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()));
    }

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
    let browser_token = headers
        .get("X-AgTalk-Browser-Token")
        .and_then(|v| v.to_str().ok());
    let workspace_root = if browser_token.is_some() {
        state.dot_agtalk.clone()
    } else {
        crate::server::handlers::workspace_root_from_headers(&headers)
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };

    crate::identity::auth::authenticate(
        &state.storage,
        &workspace_root,
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
