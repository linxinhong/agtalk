//! 图节点派发（docs/design_graph.md §5.5 / §10）：participant 解析 → msg send → assignment。
//! 独立文件以控制 handlers/graph.rs 行数在红线内。

use crate::graph::compiler::CompiledGraph;
use crate::graph::events;
use crate::graph::scheduler::DispatchItem;
use crate::graph::state::{converge_graph_run, get_node_run, transition, NodeRunStatus};
use crate::identity::auth::AuthenticatedSession;
use crate::proto::ServerMsg;
use crate::routing::lookup::lookup;
use crate::routing::send::send;
use crate::routing::SendRequest;
use crate::server::state::AppState;
use rusqlite::params;

/// 派发一个节点：解析 participant → 发消息（graph_dispatch）→ assignment 落库。
/// 无法解析/不在线/多候选 → 节点标 blocked（M1 语义）。
#[allow(clippy::result_large_err)]
pub(crate) fn dispatch_one(
    state: &AppState,
    from: &AuthenticatedSession,
    compiled: &CompiledGraph,
    item: &DispatchItem,
) -> Result<(), ServerMsg> {
    let Some(participant) = &item.participant_id else {
        // approval 节点不找 participant：复用 human approval 仲裁（design_graph.md §5.2 取舍 3）
        if item.node_type == crate::graph::spec::NodeType::Approval {
            return crate::graph::approval::dispatch_approval(state, from, compiled, item);
        }
        block_node(
            state,
            item,
            "no_participant",
            "节点未指定 participant（M1 暂不支持 Runtime 自执行 deterministic 节点）",
        )?;
        return Ok(());
    };

    let mailboxes = lookup(&state.storage, Some(participant))
        .map_err(|e| err("graph_lookup_failed", e.to_string()))?;
    let active: Vec<_> = mailboxes.iter().filter(|m| m.left_at.is_none()).collect();
    match active.len() {
        0 => block_node(
            state,
            item,
            "participant_offline",
            format!("participant '{participant}' 不在线"),
        )?,
        1 => {
            // 注意锁纪律：send()/lookup() 内部自取 storage 锁，本函数不得同时持锁调用。
            // 先无锁完成消息发送，再单独持锁写 assignment（分段持锁）。
            let target = active[0];
            let ws_info = {
                let conn = state.storage.conn();
                let spec_node = compiled
                    .nodes
                    .iter()
                    .find(|n| n.id == item.node_key)
                    .ok_or_else(|| {
                        err(
                            "graph_node_not_found",
                            format!("spec 无节点 {}", item.node_key),
                        )
                    })?;
                if spec_node.write_paths.is_empty() {
                    None
                } else {
                    // repository/base_revision 从 graph_runs 读（spec 或 CLI 探测）
                    let (repo, base): (Option<String>, Option<String>) = conn
                        .query_row(
                            "SELECT repository, base_revision FROM graph_runs WHERE id=?1",
                            params![item.graph_run_id],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .map_err(sqlite_err)?;
                    let repo = repo.unwrap_or_default();
                    if repo.is_empty() {
                        // 降级：无本地 git 仓库时不建 worktree（隔离失效，验证仍生效），记录 warning
                        crate::graph::events::append(
                            &conn,
                            &item.graph_run_id,
                            "workspace_skipped",
                            Some(&item.node_key),
                            &serde_json::json!({
                                "reason": "repository_missing",
                                "hint": "提交 spec 时自动探测本地 git 仓库；写节点建议在 git 仓库内运行",
                            }),
                        )
                        .map_err(graph_err)?;
                        None
                    } else {
                        let base = base.unwrap_or_else(|| "HEAD".into());
                        let ws = crate::graph::workspace::ensure_worktree(
                            &conn,
                            &item.graph_run_id,
                            &item.node_key,
                            &repo,
                            &base,
                        )
                        .map_err(|e| err("graph_workspace_failed", e))?;
                        Some((ws.path.unwrap_or_default(), ws.branch.unwrap_or_default()))
                    }
                }
            };
            let body = serde_json::to_string(&dispatch_payload(compiled, item, ws_info.as_ref()))
                .map_err(json_err)?;
            let subject = format!("[graph] {}", item.node_key);
            let req = SendRequest {
                to: &target.address,
                to_name: &target.name,
                from: &from.address,
                from_name: &from.name,
                body: &body,
                content_type: "graph_dispatch",
                reply_to_id: None,
                subject: Some(&subject),
                metadata: "{}",
                more_coming: false,
            };
            let msg =
                send(&state.storage, req).map_err(|e| err("graph_dispatch_send", e.to_string()))?;
            // 派发提醒（Tim 评审偏差②：设计要求派发 + notify 打扰 participant）
            {
                // 派发提醒：有 tokio runtime（HTTP handler 上下文）才触发；
                // 同步/测试上下文跳过（notify 是打扰增强，非投递必需）
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    let limiter = state.notify_limiter.clone();
                    let storage = state.storage.clone();
                    let to = target.address.clone();
                    let from_name = from.name.clone();
                    let message_id = msg.id.clone();
                    handle.spawn(async move {
                        let _ = crate::notify::trigger(
                            &storage,
                            &to,
                            &from_name,
                            &message_id,
                            &limiter,
                            Some(true),
                        )
                        .await;
                    });
                }
            }
            let conn = state.storage.conn();
            // lease：派发即记租约（reconciler 按此判定超时）
            conn.execute(
                "UPDATE node_runs SET lease_expires_at=?1 WHERE id=?2",
                params![
                    unix_now() + crate::graph::reconciler::DEFAULT_LEASE_SECONDS,
                    item.node_run_id
                ],
            )
            .map_err(sqlite_err)?;
            conn.execute(
                "INSERT INTO graph_node_assignments (id, node_run_id, message_id, kind) \
                 VALUES (?1,?2,?3,'dispatch')",
                params![uuid::Uuid::new_v4().to_string(), item.node_run_id, msg.id],
            )
            .map_err(sqlite_err)?;
            events::append(
                &conn,
                &item.graph_run_id,
                "assignment_dispatched",
                Some(&item.node_key),
                &serde_json::json!({ "message_id": msg.id }),
            )
            .map_err(graph_err)?;
        }
        _ => block_node(
            state,
            item,
            "participant_ambiguous",
            format!("participant '{participant}' 有多个活跃 mailbox，无法消歧"),
        )?,
    }
    Ok(())
}

/// 派发消息体（docs/graph-participant-protocol.md §5）。
fn dispatch_payload(
    compiled: &CompiledGraph,
    item: &DispatchItem,
    ws_info: Option<&(String, String)>,
) -> serde_json::Value {
    let spec_node = compiled
        .nodes
        .iter()
        .find(|n| n.id == item.node_key)
        .expect("execution_order 中的节点必在 nodes 中");
    serde_json::json!({
        "node_run": {
            "graph_run_id": item.graph_run_id,
            "node_key": item.node_key,
            "attempt": item.attempt,
            "node_type": item.node_type.as_str(),
            "goal": compiled.goal,
            "workspace": ws_info
                .map(|(path, branch)| serde_json::json!({ "path": path, "branch": branch })),
            "read_paths": spec_node.read_paths,
            "write_paths": spec_node.write_paths,
            "forbidden_paths": spec_node.forbidden_paths,
            "acceptance": spec_node.acceptance,
            "timeout_seconds": spec_node.timeout_seconds,
            "outputs": spec_node.outputs,
        }
    })
}

/// 派发失败 → 节点标 blocked（保留现场，等待人工/重规划）。
#[allow(clippy::result_large_err)]
fn block_node(
    state: &AppState,
    item: &DispatchItem,
    failure_type: &str,
    detail: impl Into<String>,
) -> Result<(), ServerMsg> {
    let conn = state.storage.conn();
    let run = get_node_run(&conn, &item.node_run_id)
        .map_err(graph_err)?
        .ok_or_else(|| err("graph_node_not_found", "节点不存在"))?;
    let _ = transition(
        &conn,
        &run.id,
        run.version,
        NodeRunStatus::Dispatched,
        NodeRunStatus::Blocked,
        Some(failure_type),
        Some(&detail.into()),
        None,
    )
    .map_err(graph_err)?;
    let _ = converge_graph_run(&conn, &item.graph_run_id).map_err(graph_err)?;
    Ok(())
}

fn err(code: &str, message: impl Into<String>) -> ServerMsg {
    ServerMsg::Error {
        code: code.to_string(),
        message: message.into(),
    }
}

fn sqlite_err(e: rusqlite::Error) -> ServerMsg {
    err("graph_db_error", e.to_string())
}

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
