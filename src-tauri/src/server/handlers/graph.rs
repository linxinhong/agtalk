//! 图工程 HTTP handler（docs/design_graph.md §6）。
//!
//! 职责：submit（编译建图 + 派发）、heartbeat / result（agent 上报）、
//! runs / events / control（查询与控制）。认证：写入端点走 authenticate_req（agent），
//! 读取端点额外接受 human token（图管理界面，design_graph.md §1.2）。

use super::graph_dispatch;
use crate::graph::compiler::{CompileIssue, CompiledGraph};
use crate::graph::dto::{GraphCompileIssue, GraphEventDto, GraphNodeDetail, GraphRunSummary};
use crate::graph::events;
use crate::graph::scheduler::tick;
use crate::graph::spec::GraphSpec;
use crate::graph::state::{
    converge_graph_run, get_node_run_by_key, list_node_runs, transition, NodeRunStatus,
};
use crate::identity::auth::AuthenticatedSession;
use crate::proto::ServerMsg;
use crate::server::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;

// ---- 请求体 ----

#[derive(Deserialize)]
pub struct SubmitBody {
    pub spec: String,
}

#[derive(Deserialize)]
pub struct NodeBody {
    pub run_id: String,
    pub node_key: String,
    pub attempt: u32,
    #[serde(default)]
    pub result: String,
    #[serde(default)]
    pub changed_files: Vec<String>,
    #[serde(default)]
    pub output_artifacts: Vec<crate::graph::dto::GraphArtifactRef>,
    #[serde(default)]
    pub verification_claims: Vec<crate::graph::dto::GraphClaim>,
    #[serde(default)]
    pub blockers: Vec<String>,
    /// 半完成警告（Tim 评审 P0-3）：如"遗留 TODO""有已知限制"，daemon 留证不阻塞。
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Deserialize)]
pub struct ControlBody {
    pub action: String,
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Deserialize)]
pub struct EventsQuery {
    #[serde(default)]
    pub since: Option<i64>,
}

// ---- HTTP handlers ----

pub async fn graph_submit_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SubmitBody>,
) -> (StatusCode, Json<ServerMsg>) {
    let from = match super::authenticate_req(&state, &headers) {
        Ok(s) => s,
        Err(e) => return json_response(e),
    };
    respond(submit_and_start(&state, &from, &body.spec))
}

pub async fn graph_runs_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> (StatusCode, Json<ServerMsg>) {
    if let Err(e) = authenticate_read(&state, &headers) {
        return json_response(e);
    }
    respond(list_runs(&state, q.status.as_deref()))
}

pub async fn graph_run_show_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> (StatusCode, Json<ServerMsg>) {
    if let Err(e) = authenticate_read(&state, &headers) {
        return json_response(e);
    }
    respond(show_run(&state, &run_id))
}

pub async fn graph_run_events_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
    Query(q): Query<EventsQuery>,
) -> (StatusCode, Json<ServerMsg>) {
    if let Err(e) = authenticate_read(&state, &headers) {
        return json_response(e);
    }
    respond(run_events(&state, &run_id, q.since))
}

pub async fn graph_run_control_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
    Json(body): Json<ControlBody>,
) -> (StatusCode, Json<ServerMsg>) {
    if let Err(e) = super::authenticate_req(&state, &headers) {
        return json_response(e);
    }
    respond(control_run(&state, &run_id, &body.action))
}

pub async fn graph_collab_send_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CollabSendBody>,
) -> (StatusCode, Json<ServerMsg>) {
    let from = match super::authenticate_req(&state, &headers) {
        Ok(f) => f,
        Err(e) => return json_response(e),
    };
    respond(collab_send(&state, &from, &body))
}

pub async fn graph_run_delete_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> (StatusCode, Json<ServerMsg>) {
    if let Err(e) = super::authenticate_req(&state, &headers) {
        return json_response(e);
    }
    respond(delete_run(&state, &run_id))
}

pub async fn graph_run_patch_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
    Json(body): Json<SubmitBody>,
) -> (StatusCode, Json<ServerMsg>) {
    if let Err(e) = super::authenticate_req(&state, &headers) {
        return json_response(e);
    }
    respond(patch_run(&state, &run_id, &body.spec))
}

pub async fn graph_node_heartbeat_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<NodeBody>,
) -> (StatusCode, Json<ServerMsg>) {
    if let Err(e) = super::authenticate_req(&state, &headers) {
        return json_response(e);
    }
    respond(apply_heartbeat(&state, &body))
}

pub async fn graph_node_result_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<NodeBody>,
) -> (StatusCode, Json<ServerMsg>) {
    let from = match super::authenticate_req(&state, &headers) {
        Ok(s) => s,
        Err(e) => return json_response(e),
    };
    respond(apply_result(&state, &from, &body))
}

fn json_response(resp: ServerMsg) -> (StatusCode, Json<ServerMsg>) {
    let status = super::status_for(&resp);
    (status, Json(resp))
}

/// 把领域层 Result 折叠为响应。
fn respond(res: Result<ServerMsg, ServerMsg>) -> (StatusCode, Json<ServerMsg>) {
    json_response(res.unwrap_or_else(|e| e))
}

/// flush 新增 GraphEvent 到推送中枢（先落库后推送，design_graph.md §11 红线 6）。
/// `since` 为函数开始时的 max_event_id；期间所有 append（含状态机内部事件）都会被推送。
pub(crate) fn push_new_graph_events(state: &AppState, run_id: &str, since: i64) {
    let events = {
        let conn = state.storage.conn();
        crate::graph::events::list_after(&conn, run_id, Some(since), 200).unwrap_or_default()
    };
    for e in events {
        let dto = crate::graph::dto::GraphEventDto {
            id: e.id,
            event_type: e.event_type,
            node_key: e.node_key,
            payload: e.payload,
            created_at: e.created_at,
        };
        state.graph_events.push(run_id, dto);
    }
}

/// 当前 run 的最大事件 id（flush 起点）。
pub(crate) fn graph_max_event_id(state: &AppState, run_id: &str) -> i64 {
    let conn = state.storage.conn();
    crate::graph::events::max_event_id(&conn, run_id).unwrap_or(0)
}

// ---- 领域逻辑 ----

/// 读取端点认证：human token 或 agent（design_graph.md §6）。
#[allow(clippy::result_large_err)]
pub(crate) fn authenticate_read(state: &AppState, headers: &HeaderMap) -> Result<(), ServerMsg> {
    if let Some(token) = headers
        .get("X-AgTalk-Human-Token")
        .and_then(|v| v.to_str().ok())
    {
        crate::identity::human_session::validate(token)
            .map(|_| ())
            .map_err(|_| super::auth_error("human token 无效".into()))
    } else {
        super::authenticate_req(state, headers).map(|_| ())
    }
}

/// 提交 spec：解析 → 编译 → 建 GraphRun/NodeRun → 派发第一批（docs/design_graph.md §13 步骤 1-6）。
#[allow(clippy::result_large_err)]
pub fn submit_and_start(
    state: &AppState,
    from: &AuthenticatedSession,
    spec_yaml: &str,
) -> Result<ServerMsg, ServerMsg> {
    let spec = GraphSpec::parse(spec_yaml)
        .map_err(|e| err("graph_spec_parse_error", format!("spec 解析失败: {e}")))?;
    let compiled = crate::graph::compiler::compile(&spec);
    if !compiled.valid {
        return Ok(ServerMsg::GraphRunCreated {
            run_id: String::new(),
            status: "invalid".into(),
            errors: issues(&compiled.errors),
            warnings: issues(&compiled.warnings),
        });
    }
    // participant=auto / 缺省 → 自动分配随机执行者名（2 字中文代号，GUI 头像按 djb2 自动映射）
    let mut compiled = compiled;
    let mut cg = compiled.compiled.clone().expect("valid 编译必有 compiled");
    for node in &mut cg.nodes {
        let need = match node.executor_requirements.participant.as_deref() {
            Some(p) => p == "auto",
            None => node.node_type == crate::graph::spec::NodeType::Executor,
        };
        if need {
            let name = crate::identity::namegen::random_agent_name();
            node.executor_requirements.participant = Some(name.clone());
            compiled
                .warnings
                .push(crate::graph::compiler::CompileIssue {
                    code: "auto_assigned",
                    message: format!("节点 '{}' 自动分配执行者：{}", node.id, name),
                    node: Some(node.id.clone()),
                });
        }
    }
    let run_id = uuid::Uuid::new_v4().to_string();
    let now = unix_now();
    let since = graph_max_event_id(state, &run_id);

    {
        let mut conn = state.storage.conn();
        let tx = conn.transaction().map_err(sqlite_err)?;
        tx.execute(
            "INSERT INTO graph_runs (id, goal, spec_snapshot, compiled_graph, status, \
             repository, base_revision, integration_target, created_at, submitter_name, submitter_address) \
             VALUES (?1,?2,?3,?4,'ready',?5,?6,?7,?8,?9,?10)",
            params![
                run_id,
                cg.goal,
                serde_json::to_string(&spec).map_err(json_err)?,
                serde_json::to_string(&cg).map_err(json_err)?,
                cg.repository,
                cg.base_revision,
                cg.integration_target,
                now,
                from.name,
                from.address
            ],
        )
        .map_err(sqlite_err)?;
        for n in &cg.nodes {
            tx.execute(
                "INSERT INTO node_runs (id, graph_run_id, node_key, node_type, attempt, \
                 participant_id, workspace_id) VALUES (?1,?2,?3,?4,1,?5,?6)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    run_id,
                    n.id,
                    n.node_type.as_str(),
                    n.executor_requirements.participant,
                    n.workspace
                ],
            )
            .map_err(sqlite_err)?;
        }
        tx.commit().map_err(sqlite_err)?;
    }

    // 事件 + 派发候选计算（作用域内持锁）
    let items = {
        let conn = state.storage.conn();
        events::append(
            &conn,
            &run_id,
            "graph_created",
            None,
            &serde_json::json!({}),
        )
        .map_err(graph_err)?;
        events::append(
            &conn,
            &run_id,
            "graph_validated",
            None,
            &serde_json::json!({ "warnings": compiled.warnings.len() }),
        )
        .map_err(graph_err)?;
        tick(&conn, &run_id, &cg).map_err(graph_err)?
    };
    // 派发第一批就绪节点（无锁态：dispatch_one 内部自取锁调 lookup/send）
    for item in &items {
        graph_dispatch::dispatch_one(state, from, &cg, item).map_err(server_err)?;
    }
    {
        let conn = state.storage.conn();
        let _ = converge_graph_run(&conn, &run_id).map_err(graph_err)?;
    }

    push_new_graph_events(state, &run_id, since);

    Ok(ServerMsg::GraphRunCreated {
        run_id,
        status: "ready".into(),
        errors: Vec::new(),
        warnings: issues(&compiled.warnings),
    })
}

/// heartbeat：dispatched → running（首次上报）；running → 更新心跳时间。
#[allow(clippy::result_large_err)]
pub fn apply_heartbeat(state: &AppState, body: &NodeBody) -> Result<ServerMsg, ServerMsg> {
    let since = graph_max_event_id(state, &body.run_id);
    let run = {
        let conn = state.storage.conn();
        get_node_run_by_key(&conn, &body.run_id, &body.node_key, body.attempt)
            .map_err(graph_err)?
            .ok_or_else(|| err("graph_node_not_found", "节点不存在"))?
    };
    let now = unix_now();
    match run.status {
        NodeRunStatus::Dispatched => {
            let conn = state.storage.conn();
            transition(
                &conn,
                &run.id,
                run.version,
                NodeRunStatus::Dispatched,
                NodeRunStatus::Running,
                None,
                None,
                Some(now),
            )
            .map_err(graph_err)?;
        }
        NodeRunStatus::Running => {
            // 心跳续租：更新 heartbeat_at + lease_expires_at
            let conn = state.storage.conn();
            conn.execute(
                "UPDATE node_runs SET heartbeat_at=?1, lease_expires_at=?2 WHERE id=?3",
                params![
                    now,
                    now + crate::graph::reconciler::DEFAULT_LEASE_SECONDS,
                    run.id
                ],
            )
            .map_err(sqlite_err)?;
        }
        _ => {
            return Err(err(
                "graph_invalid_node_state",
                format!("节点状态 {} 不能上报 heartbeat", run.status.as_str()),
            ));
        }
    }
    push_new_graph_events(state, &body.run_id, since);
    Ok(ServerMsg::GraphNodeReportOk {
        run_id: body.run_id.clone(),
        node_key: body.node_key.clone(),
        attempt: body.attempt,
        status: "running".into(),
        message: "heartbeat 已记录".into(),
    })
}

/// result：running → verifying（claims 落库）→ succeeded（M1 无独立验证，信任自证留证）。
/// blockers 非空 → running → blocked。
#[allow(clippy::result_large_err)]
pub fn apply_result(
    state: &AppState,
    from: &AuthenticatedSession,
    body: &NodeBody,
) -> Result<ServerMsg, ServerMsg> {
    let since = graph_max_event_id(state, &body.run_id);
    // blockers 非空 → blocked（独立作用域，design_graph.md §八 semantic_blocker）
    if !body.blockers.is_empty() {
        {
            let conn = state.storage.conn();
            let run = get_node_run_by_key(&conn, &body.run_id, &body.node_key, body.attempt)
                .map_err(graph_err)?
                .ok_or_else(|| err("graph_node_not_found", "节点不存在"))?;
            if run.status != NodeRunStatus::Running {
                return Err(err(
                    "graph_invalid_node_state",
                    format!("节点状态 {} 不能提交结果", run.status.as_str()),
                ));
            }
            transition(
                &conn,
                &run.id,
                run.version,
                NodeRunStatus::Running,
                NodeRunStatus::Blocked,
                Some("semantic_blocker"),
                Some(&body.blockers.join("; ")),
                None,
            )
            .map_err(graph_err)?;
            // P0-3：result warnings 留证（半完成状态，事件可见不阻塞）
            if !body.warnings.is_empty() {
                events::append(
                    &conn,
                    &body.run_id,
                    "node_warnings",
                    Some(&body.node_key),
                    &serde_json::json!({ "warnings": body.warnings }),
                )
                .map_err(graph_err)?;
            }
            let _ = converge_graph_run(&conn, &body.run_id).map_err(graph_err)?;
        }
        push_new_graph_events(state, &body.run_id, since);
        return Ok(ServerMsg::GraphNodeReportOk {
            run_id: body.run_id.clone(),
            node_key: body.node_key.clone(),
            attempt: body.attempt,
            status: "blocked".into(),
            message: "已记录 blocker，节点阻塞".into(),
        });
    }

    // M2 验证 + 状态推进（锁内）
    let outcome = crate::server::handlers::graph_verify::verify_and_advance(state, body)?;
    // 派发下游/重试候选（无锁态：dispatch_one 内部自取锁）
    for item in &outcome.items {
        graph_dispatch::dispatch_one(state, from, &outcome.cg, item).map_err(server_err)?;
    }
    {
        let conn = state.storage.conn();
        let _ = converge_graph_run(&conn, &body.run_id).map_err(graph_err)?;
    }
    push_new_graph_events(state, &body.run_id, since);

    Ok(ServerMsg::GraphNodeReportOk {
        run_id: body.run_id.clone(),
        node_key: body.node_key.clone(),
        attempt: body.attempt,
        status: outcome.node_status,
        message: outcome.message,
    })
}

/// 运行列表（按状态过滤）。
#[allow(clippy::result_large_err)]
pub fn list_runs(state: &AppState, status: Option<&str>) -> Result<ServerMsg, ServerMsg> {
    let conn = state.storage.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, goal, status, repository, base_revision, created_at, started_at, \
             completed_at, failure_reason FROM graph_runs \
             WHERE (?1 IS NULL OR status=?1) ORDER BY created_at DESC",
        )
        .map_err(sqlite_err)?;
    let runs = stmt
        .query_map(params![status], summary_from_row)
        .map_err(sqlite_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sqlite_err)?;
    Ok(ServerMsg::GraphRunList { runs })
}

/// 运行详情：summary + 全部 NodeRun（含各 attempt）。
#[allow(clippy::result_large_err)]
pub fn show_run(state: &AppState, run_id: &str) -> Result<ServerMsg, ServerMsg> {
    let conn = state.storage.conn();
    let summary = conn
        .query_row(
            "SELECT id, goal, status, repository, base_revision, created_at, started_at, \
             completed_at, failure_reason FROM graph_runs WHERE id=?1",
            params![run_id],
            summary_from_row,
        )
        .optional()
        .map_err(sqlite_err)?
        .ok_or_else(|| err("graph_run_not_found", "GraphRun 不存在"))?;

    let runs = list_node_runs(&conn, run_id).map_err(graph_err)?;
    let nodes = runs
        .iter()
        .map(|r| GraphNodeDetail {
            node_key: r.node_key.clone(),
            node_type: r.node_type.clone(),
            status: r.status.as_str().to_string(),
            attempt: r.attempt,
            participant_id: r.participant_id.clone(),
            // 认领状态：participant 有活跃 mailbox（join 且未 leave）即"已认领"
            participant_online: r.participant_id.as_ref().is_some_and(|p| {
                conn.query_row(
                    "SELECT COUNT(*) FROM mailboxes WHERE name=?1 AND left_at IS NULL",
                    params![p],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0)
                    > 0
            }),
            workspace_id: r.workspace_id.clone(),
            started_at: r.started_at,
            completed_at: r.completed_at,
            failure_type: r.failure_type.clone(),
            failure_detail: r.failure_detail.clone(),
        })
        .collect();

    // required_approvals / resource_conflicts 从 compiled_graph JSON 读
    let cg_json: String = conn
        .query_row(
            "SELECT compiled_graph FROM graph_runs WHERE id=?1",
            params![run_id],
            |r| r.get(0),
        )
        .map_err(sqlite_err)?;
    let cg: CompiledGraph = serde_json::from_str(&cg_json).map_err(json_err)?;

    let edges = cg
        .edges
        .iter()
        .map(|e| crate::graph::dto::GraphEdgeDto {
            from: e.from.clone(),
            to: e.to.clone(),
            trigger: e.trigger.as_str().to_string(),
        })
        .collect();

    Ok(ServerMsg::GraphRunDetail {
        run: summary,
        nodes,
        edges,
        required_approvals: cg.required_approvals,
        resource_conflicts: cg.resource_conflicts,
    })
}

/// 事件日志（since 之后，SSE 重放语义）。
#[allow(clippy::result_large_err)]
pub fn run_events(
    state: &AppState,
    run_id: &str,
    since: Option<i64>,
) -> Result<ServerMsg, ServerMsg> {
    let conn = state.storage.conn();
    let events = events::list_after(&conn, run_id, since, 500).map_err(graph_err)?;
    let dto: Vec<GraphEventDto> = events
        .into_iter()
        .map(|e| GraphEventDto {
            id: e.id,
            event_type: e.event_type,
            node_key: e.node_key,
            payload: e.payload,
            created_at: e.created_at,
        })
        .collect();
    Ok(ServerMsg::GraphEventsResult { events: dto })
}

/// Graph Patch（P2）：以完整新 spec 替换图定义并重新编译校验。
/// 仅允许 paused/ready 状态图应用；running 需先 pause，terminal 拒绝。
/// 运行现场（node_runs/workspaces）保留；新节点需重新提交完整图（或后续扩展增量）。
#[allow(clippy::result_large_err)]
pub fn patch_run(state: &AppState, run_id: &str, spec_yaml: &str) -> Result<ServerMsg, ServerMsg> {
    let spec = GraphSpec::parse(spec_yaml)
        .map_err(|e| err("graph_spec_parse_error", format!("spec 解析失败: {e}")))?;
    let compiled = crate::graph::compiler::compile(&spec);
    if !compiled.valid {
        return Ok(ServerMsg::GraphRunCreated {
            run_id: String::new(),
            status: "invalid".into(),
            errors: issues(&compiled.errors),
            warnings: issues(&compiled.warnings),
        });
    }
    let cg = compiled
        .compiled
        .clone()
        .ok_or_else(|| err("graph_compile_error", "编译无结果"))?;
    let conn = state.storage.conn();
    let status: String = conn
        .query_row(
            "SELECT status FROM graph_runs WHERE id=?1",
            params![run_id],
            |r| r.get(0),
        )
        .map_err(sqlite_err)?;
    if matches!(status.as_str(), "completed" | "failed" | "cancelled") {
        return Err(err(
            "graph_patch_terminal",
            format!("图已 {status}，不能 patch"),
        ));
    }
    if status == "running" {
        return Err(err(
            "graph_patch_running",
            "运行中的图不能 patch（先 pause 或 cancel）",
        ));
    }
    conn.execute(
        "UPDATE graph_runs SET goal=?1, spec_snapshot=?2, compiled_graph=?3 WHERE id=?4",
        params![
            cg.goal,
            serde_json::to_string(&spec).map_err(json_err)?,
            serde_json::to_string(&cg).map_err(json_err)?,
            run_id
        ],
    )
    .map_err(sqlite_err)?;
    events::append(&conn, run_id, "graph_patched", None, &serde_json::json!({}))
        .map_err(graph_err)?;
    Ok(ServerMsg::GraphRunControlOk {
        run_id: run_id.to_string(),
        action: "patch".into(),
        status: "patched".into(),
    })
}

/// 物理删除 GraphRun（级联清理子表）。仅 terminal 状态（cancelled/completed/failed）可删；
/// 运行中需先 cancel。删除后不可恢复。
#[allow(clippy::result_large_err)]
pub fn delete_run(state: &AppState, run_id: &str) -> Result<ServerMsg, ServerMsg> {
    let conn = state.storage.conn();
    let status: String = conn
        .query_row(
            "SELECT status FROM graph_runs WHERE id=?1",
            params![run_id],
            |r| r.get(0),
        )
        .map_err(sqlite_err)?;
    if !matches!(status.as_str(), "cancelled" | "completed" | "failed") {
        return Err(err(
            "graph_delete_non_terminal",
            format!("图状态 {status} 不能删除（请先 cancel）"),
        ));
    }
    // 级联删除（FK 无 CASCADE，按依赖序清理子表）
    let tx = conn.unchecked_transaction().map_err(sqlite_err)?;
    for node_id in {
        let mut stmt = tx
            .prepare("SELECT id FROM node_runs WHERE graph_run_id=?1")
            .map_err(sqlite_err)?;
        let rows: Vec<String> = stmt
            .query_map(params![run_id], |r| r.get(0))
            .map_err(sqlite_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sqlite_err)?;
        rows
    } {
        tx.execute(
            "DELETE FROM graph_node_assignments WHERE node_run_id=?1",
            params![node_id],
        )
        .map_err(sqlite_err)?;
        tx.execute(
            "DELETE FROM verifications WHERE node_run_id=?1",
            params![node_id],
        )
        .map_err(sqlite_err)?;
    }
    tx.execute(
        "DELETE FROM node_runs WHERE graph_run_id=?1",
        params![run_id],
    )
    .map_err(sqlite_err)?;
    tx.execute(
        "DELETE FROM artifacts WHERE graph_run_id=?1",
        params![run_id],
    )
    .map_err(sqlite_err)?;
    tx.execute(
        "DELETE FROM workspaces WHERE graph_run_id=?1",
        params![run_id],
    )
    .map_err(sqlite_err)?;
    tx.execute(
        "DELETE FROM graph_events WHERE graph_run_id=?1",
        params![run_id],
    )
    .map_err(sqlite_err)?;
    tx.execute("DELETE FROM graph_runs WHERE id=?1", params![run_id])
        .map_err(sqlite_err)?;
    tx.commit().map_err(sqlite_err)?;
    // GC：artifact store 目录随 run 删除清理（Tim 失败项1：GC 生命周期保证）
    if let Ok(root) = crate::paths::artifact_store_root() {
        let dir = root.join(run_id);
        let _ = std::fs::remove_dir_all(&dir);
    }
    Ok(ServerMsg::GraphRunDeleted {
        run_id: run_id.to_string(),
    })
}

/** collab 请求体（proto ClientMsg::GraphCollabSend 的字段镜像，供 axum 直接反序列化）。 */
#[derive(serde::Deserialize)]
pub struct CollabSendBody {
    pub graph_run_id: String,
    pub node_key: String,
    pub to_address: String,
    pub kind: String,
    pub question: String,
    #[serde(default)]
    pub depth: u32,
    #[serde(default)]
    pub context_artifacts: Vec<String>,
}

/** M3：节点协作消息（Tim 评审）——graph_collab 走 graph 自包含端点。
 *  元数据（run/node/kind/depth）落 node_collab 事件可审计；消息正文 body 自由（点对点）。 */
#[allow(clippy::result_large_err)]
pub fn collab_send(
    state: &AppState,
    from: &AuthenticatedSession,
    req: &CollabSendBody,
) -> Result<ServerMsg, ServerMsg> {
    // 校验图存在
    {
        let conn = state.storage.conn();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM graph_runs WHERE id=?1",
                params![req.graph_run_id],
                |r| r.get(0),
            )
            .map_err(sqlite_err)?;
        if n == 0 {
            return Err(err("graph_run_not_found", "GraphRun 不存在"));
        }
    }
    // depth 防循环（Tim 补强）：≥3 拒绝（应转 escalation/blockers）
    if req.depth >= 3 {
        return Err(err(
            "graph_collab_depth_limit",
            "collab depth≥3：请转 escalation 或 result blockers（防循环咨询）",
        ));
    }
    let subject = format!(
        "[collab] {} {} {} {}",
        req.graph_run_id, req.node_key, req.kind, req.depth
    );
    let body = serde_json::json!({
        "graph_run_id": req.graph_run_id,
        "node": req.node_key,
        "kind": req.kind,
        "depth": req.depth,
        "question": req.question,
        "context_artifacts": req.context_artifacts,
        "from": { "name": from.name, "address": from.address },
    });
    let send_req = crate::routing::SendRequest {
        to: &req.to_address,
        to_name: "",
        from: &from.address,
        from_name: &from.name,
        body: &serde_json::to_string(&body).map_err(json_err)?,
        content_type: "graph_collab",
        reply_to_id: None,
        subject: Some(&subject),
        metadata: "{}",
        more_coming: false,
    };
    let msg = crate::routing::send::send(&state.storage, send_req)
        .map_err(|e| err("graph_collab_send_failed", e.to_string()))?;
    // 留证（node_collab 事件）
    {
        let conn = state.storage.conn();
        events::append(
            &conn,
            &req.graph_run_id,
            "node_collab",
            Some(&req.node_key),
            &serde_json::json!({
                "kind": req.kind,
                "to": req.to_address,
                "depth": req.depth,
                "message_id": msg.id,
                "question": req.question,
            }),
        )
        .map_err(graph_err)?;
    }
    Ok(ServerMsg::GraphCollabSent { message_id: msg.id })
}

/// 运行控制：cancel / pause / resume。
#[allow(clippy::result_large_err)]
pub fn control_run(state: &AppState, run_id: &str, action: &str) -> Result<ServerMsg, ServerMsg> {
    let since = graph_max_event_id(state, run_id);
    match action {
        "cancel" => {
            {
                let conn = state.storage.conn();
                let runs = list_node_runs(&conn, run_id).map_err(graph_err)?;
                for r in &runs {
                    if !r.status.is_terminal() {
                        let _ = transition(
                            &conn,
                            &r.id,
                            r.version,
                            r.status,
                            NodeRunStatus::Cancelled,
                            None,
                            None,
                            None,
                        )
                        .map_err(graph_err)?;
                    }
                }
                conn.execute(
                    "UPDATE graph_runs SET status='cancelled', completed_at=?1 WHERE id=?2",
                    params![unix_now(), run_id],
                )
                .map_err(sqlite_err)?;
                events::append(
                    &conn,
                    run_id,
                    "graph_cancelled",
                    None,
                    &serde_json::json!({}),
                )
                .map_err(graph_err)?;
            }
            push_new_graph_events(state, run_id, since);
            Ok(ServerMsg::GraphRunControlOk {
                run_id: run_id.to_string(),
                action: action.to_string(),
                status: "cancelled".into(),
            })
        }
        "pause" => {
            {
                let conn = state.storage.conn();
                conn.execute(
                    "UPDATE graph_runs SET status='paused' WHERE id=?1",
                    params![run_id],
                )
                .map_err(sqlite_err)?;
                events::append(&conn, run_id, "graph_paused", None, &serde_json::json!({}))
                    .map_err(graph_err)?;
            }
            push_new_graph_events(state, run_id, since);
            Ok(ServerMsg::GraphRunControlOk {
                run_id: run_id.to_string(),
                action: action.to_string(),
                status: "paused".into(),
            })
        }
        "resume" => {
            {
                let conn = state.storage.conn();
                conn.execute(
                    "UPDATE graph_runs SET status='running' WHERE id=?1 AND status='paused'",
                    params![run_id],
                )
                .map_err(sqlite_err)?;
                events::append(&conn, run_id, "graph_resumed", None, &serde_json::json!({}))
                    .map_err(graph_err)?;
            }
            push_new_graph_events(state, run_id, since);
            Ok(ServerMsg::GraphRunControlOk {
                run_id: run_id.to_string(),
                action: action.to_string(),
                status: "running".into(),
            })
        }
        _ => Err(err(
            "graph_unknown_action",
            format!("未知控制动作: {action}"),
        )),
    }
}

// ---- 工具 ----

fn summary_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<GraphRunSummary> {
    Ok(GraphRunSummary {
        id: r.get(0)?,
        goal: r.get(1)?,
        status: r.get(2)?,
        repository: r.get(3)?,
        base_revision: r.get(4)?,
        created_at: r.get(5)?,
        started_at: r.get(6)?,
        completed_at: r.get(7)?,
        failure_reason: r.get(8)?,
    })
}

fn issues(v: &[CompileIssue]) -> Vec<GraphCompileIssue> {
    v.iter()
        .map(|i| GraphCompileIssue {
            code: i.code.to_string(),
            message: i.message.clone(),
            node: i.node.clone(),
        })
        .collect()
}

fn err(code: &str, message: impl Into<String>) -> ServerMsg {
    ServerMsg::Error {
        code: code.to_string(),
        message: message.into(),
    }
}

fn server_err(e: ServerMsg) -> ServerMsg {
    e
}

fn sqlite_err(e: rusqlite::Error) -> ServerMsg {
    err("graph_db_error", e.to_string())
}

/// graph 领域错误 → ServerMsg。
fn graph_err(e: crate::graph::Error) -> ServerMsg {
    match e {
        crate::graph::Error::Sqlite(e) => err("graph_db_error", e.to_string()),
        crate::graph::Error::Json(e) => err("graph_json_error", e.to_string()),
        other => err("graph_error", other.to_string()),
    }
}

fn json_err(e: serde_json::Error) -> ServerMsg {
    err("graph_json_error", e.to_string())
}

fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}
