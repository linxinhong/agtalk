//! Reconciler（docs/design_graph.md §5.7 / M2 基础版）。
//!
//! daemon 启动时收敛运行现场：
//! 1. 对 running/paused 的图调用 converge_graph_run（终态推导）；
//! 2. lease 过期（lease_expires_at < now）且状态 active 的节点 → timed_out。
//!    避免重复执行/重复建 worktree（幂等键 UNIQUE + workspaces 先查）。

use crate::storage::Storage;
use rusqlite::{params, Connection, OptionalExtension};

/// lease 默认窗口（秒）：派发后未心跳视为过期（dispatch 未显式设置时用此兜底）。
pub const DEFAULT_LEASE_SECONDS: f64 = 300.0;

/// 启动恢复：返回处理的节点数（标 timed_out / 收敛的图数）。
pub fn reconcile_all(storage: &Storage) -> Result<usize, crate::graph::Error> {
    let conn = storage.conn();
    let mut handled = 0;

    // 1. 所有非终态图的节点：lease 过期且 active → timed_out
    let expired: Vec<(String, String, u32)> = conn
        .prepare(
            "SELECT id, graph_run_id, version FROM node_runs \
             WHERE status IN ('leased','dispatched','running','verifying') \
               AND lease_expires_at IS NOT NULL AND lease_expires_at < ?1",
        )?
        .query_map(params![unix_now()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u32>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    for (node_id, graph_run_id, version) in expired {
        let status = current_status(&conn, &node_id)?;
        let from = crate::graph::state::NodeRunStatus::parse_str(&status)
            .unwrap_or(crate::graph::state::NodeRunStatus::Leased);
        // 只处理可超时的状态（状态机允许 → timed_out）
        if matches!(
            from,
            crate::graph::state::NodeRunStatus::Leased
                | crate::graph::state::NodeRunStatus::Dispatched
                | crate::graph::state::NodeRunStatus::Running
        ) {
            let _ = crate::graph::state::transition(
                &conn,
                &node_id,
                version,
                from,
                crate::graph::state::NodeRunStatus::TimedOut,
                Some("lease_expired"),
                Some("daemon 重启后 lease 过期，节点超时"),
                None,
            )?;
            let _ = crate::graph::state::converge_graph_run(&conn, &graph_run_id)?;
            handled += 1;
            // 超时重派（design_graph.md §八：agent_timeout 可重试且未超 max_attempts → 新 attempt）
            if let Some((node_key, attempt)) = node_identity(&conn, &node_id)? {
                if retry_timed_out(&conn, &graph_run_id, &node_key, attempt)? {
                    handled += 1;
                }
            }
        }
    }

    // 2. running/paused 图收敛（节点已全部 terminal 但图状态未更新的情况）
    let running_graphs: Vec<String> = conn
        .prepare("SELECT id FROM graph_runs WHERE status IN ('running','paused','ready')")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for graph_id in running_graphs {
        let _ = crate::graph::state::converge_graph_run(&conn, &graph_id)?;
    }

    // 3. 脏 Workspace 恢复检查（P2）：worktree 有未提交改动 → 保留现场 + 事件（不自动清理）
    //    design_graph.md §六.4：节点失败后现场默认保留；此处覆盖 daemon 重启后未提交 diff
    let active_graphs: Vec<String> = conn
        .prepare("SELECT id FROM graph_runs WHERE status IN ('running','paused')")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for graph_id in active_graphs {
        let wss = crate::graph::workspace::list_by_graph(&conn, &graph_id)?;
        for ws in wss {
            let Some(path) = &ws.path else { continue };
            let p = std::path::Path::new(path);
            if !p.exists() {
                continue; // worktree 已被移除（非现场保留场景）
            }
            let dirty =
                crate::graph::workspace::git(p, &["status", "--porcelain"]).unwrap_or_default();
            if !dirty.trim().is_empty() {
                conn.execute(
                    "UPDATE workspaces SET dirty=1 WHERE id=?1",
                    rusqlite::params![ws.id],
                )?;
                crate::graph::events::append(
                    &conn,
                    &graph_id,
                    "workspace_dirty",
                    None,
                    &serde_json::json!({
                        "workspace": ws.id,
                        "branch": ws.branch,
                        "path": path,
                        "hint": "未提交改动已保留（失败现场），需要人工决定提交/放弃",
                    }),
                )?;
                handled += 1;
            }
        }
    }

    Ok(handled)
}

/// 节点的 (node_key, attempt)。
fn node_identity(
    conn: &Connection,
    node_id: &str,
) -> Result<Option<(String, u32)>, crate::graph::Error> {
    conn.query_row(
        "SELECT node_key, attempt FROM node_runs WHERE id=?1",
        params![node_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .map_err(crate::graph::Error::Sqlite)
}

/// 超时重派：spec retry_policy 允许 agent_timeout 且未超 max_attempts → 新建 attempt。
/// 返回是否创建了新 attempt（由 daemon 定时调度随后 tick 派发）。
fn retry_timed_out(
    conn: &Connection,
    graph_run_id: &str,
    node_key: &str,
    current_attempt: u32,
) -> Result<bool, crate::graph::Error> {
    let cg_json: Option<String> = conn
        .query_row(
            "SELECT compiled_graph FROM graph_runs WHERE id=?1",
            params![graph_run_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(crate::graph::Error::Sqlite)?;
    let Some(cg_json) = cg_json else {
        return Ok(false);
    };
    let Ok(cg) = serde_json::from_str::<crate::graph::compiler::CompiledGraph>(&cg_json) else {
        return Ok(false); // compiled_graph 不可解析（如旧数据占位）→ 不重试
    };
    let Some(spec_node) = cg.nodes.iter().find(|n| n.id == node_key) else {
        return Ok(false);
    };
    let rp = &spec_node.retry_policy;
    if current_attempt >= rp.max_attempts {
        return Ok(false);
    }
    if !rp.retryable.iter().any(|f| f.as_str() == "agent_timeout") {
        return Ok(false);
    }
    // 原 attempt 的 participant/workspace（node_type 用 spec 的）
    let (participant, workspace): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT participant_id, workspace_id FROM node_runs \
             WHERE graph_run_id=?1 AND node_key=?2 AND attempt=?3",
            params![graph_run_id, node_key, current_attempt],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(crate::graph::Error::Sqlite)?
        .unwrap_or((None, None));
    let next = current_attempt + 1;
    let (_, created) = crate::graph::state::create_node_run(
        conn,
        graph_run_id,
        node_key,
        spec_node.node_type,
        next,
        participant.as_deref(),
        workspace.as_deref(),
    )?;
    if created {
        crate::graph::events::append(
            conn,
            graph_run_id,
            "node_ready",
            Some(node_key),
            &serde_json::json!({ "attempt": next, "retry_of_failure": "agent_timeout" }),
        )?;
    }
    Ok(created)
}

fn current_status(conn: &Connection, node_id: &str) -> Result<String, crate::graph::Error> {
    conn.query_row(
        "SELECT status FROM node_runs WHERE id=?1",
        params![node_id],
        |r| r.get(0),
    )
    .map_err(crate::graph::Error::Sqlite)
}

pub fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::spec::NodeType;
    use crate::graph::state::{create_node_run, NodeRunStatus};

    fn setup(storage: &Storage) -> (String, String) {
        let conn = storage.conn();
        conn.execute(
            "INSERT INTO graph_runs (id, goal, spec_snapshot, compiled_graph, status) \
             VALUES ('g1', 'goal', '{}', '{}', 'running')",
            [],
        )
        .unwrap();
        let (id, _) = create_node_run(
            &conn,
            "g1",
            "a",
            NodeType::Executor,
            1,
            Some("p1"),
            Some("w1"),
        )
        .unwrap();
        // 直接设为 dispatched + lease 已过期
        conn.execute(
            "UPDATE node_runs SET status='dispatched', lease_expires_at=?1 WHERE id=?2",
            params![unix_now() - 60.0, id],
        )
        .unwrap();
        (id, "g1".into())
    }

    #[test]
    fn expired_lease_becomes_timed_out() {
        let storage = Storage::open_in_memory().unwrap();
        let (id, graph_id) = setup(&storage);
        let n = reconcile_all(&storage).unwrap();
        assert_eq!(n, 1, "过期 lease 节点应被标记");
        let conn = storage.conn();
        let status: String = conn
            .query_row(
                "SELECT status FROM node_runs WHERE id=?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "timed_out");
        // 图收敛为 failed（唯一节点 timed_out）
        let gstatus: String = conn
            .query_row(
                "SELECT status FROM graph_runs WHERE id=?1",
                params![graph_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(gstatus, "failed");
    }

    #[test]
    fn fresh_lease_untouched() {
        let storage = Storage::open_in_memory().unwrap();
        let (id, _) = setup(&storage);
        {
            let conn = storage.conn();
            conn.execute(
                "UPDATE node_runs SET lease_expires_at=?1 WHERE id=?2",
                params![unix_now() + 600.0, id],
            )
            .unwrap();
        }
        let n = reconcile_all(&storage).unwrap();
        assert_eq!(n, 0);
        let conn = storage.conn();
        let status: String = conn
            .query_row(
                "SELECT status FROM node_runs WHERE id=?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "dispatched", "未过期 lease 不应被处理");
    }

    #[test]
    fn idle() {
        // 无过期节点时 reconcile 幂等
        let storage = Storage::open_in_memory().unwrap();
        let n = reconcile_all(&storage).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn timed_out_retries_with_new_attempt() {
        // 超时重派（design_graph.md §八）：spec retryable 含 agent_timeout → 新 attempt
        let storage = Storage::open_in_memory().unwrap();
        let yaml = r#"
version: 1
goal: "retry timeout"
nodes:
  - id: a
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p1 }
    workspace: w
    write_paths: [x]
    acceptance: [{ type: path }]
    timeout_seconds: 300
    retry_policy: { max_attempts: 2, retryable: [agent_timeout] }
"#;
        let spec = crate::graph::spec::GraphSpec::parse(yaml).unwrap();
        let cg = crate::graph::compiler::compile(&spec).compiled.unwrap();
        {
            let conn = storage.conn();
            conn.execute(
                "INSERT INTO graph_runs (id, goal, spec_snapshot, compiled_graph, status) \
                 VALUES ('g1', 'g', '{}', ?1, 'running')",
                params![serde_json::to_string(&cg).unwrap()],
            )
            .unwrap();
            let (id, _) = create_node_run(
                &conn,
                "g1",
                "a",
                NodeType::Executor,
                1,
                Some("p1"),
                Some("w1"),
            )
            .unwrap();
            conn.execute(
                "UPDATE node_runs SET status='dispatched', lease_expires_at=?1 WHERE id=?2",
                params![unix_now() - 60.0, id],
            )
            .unwrap();
        }

        let n = reconcile_all(&storage).unwrap();
        assert!(n >= 2, "timed_out(1) + 重试 attempt(1)，实际 {n}");

        let a2 = {
            let conn = storage.conn();
            crate::graph::state::get_node_run_by_key(&conn, "g1", "a", 2)
                .unwrap()
                .unwrap()
        };
        assert_eq!(a2.status, NodeRunStatus::Pending, "attempt 2 已建待派发");
        // attempt 1 保留 timed_out 记录
        let a1 = {
            let conn = storage.conn();
            crate::graph::state::get_node_run_by_key(&conn, "g1", "a", 1)
                .unwrap()
                .unwrap()
        };
        assert_eq!(a1.status, NodeRunStatus::TimedOut);
    }

    #[test]
    fn timed_out_without_retryable_stays_failed() {
        // 不可重试超时：不建新 attempt，图收敛 failed
        let storage = Storage::open_in_memory().unwrap();
        let yaml = r#"
version: 1
goal: "no retry"
nodes:
  - id: a
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p1 }
    workspace: w
    write_paths: [x]
    acceptance: [{ type: path }]
    timeout_seconds: 300
"#;
        let spec = crate::graph::spec::GraphSpec::parse(yaml).unwrap();
        let cg = crate::graph::compiler::compile(&spec).compiled.unwrap();
        {
            let conn = storage.conn();
            conn.execute(
                "INSERT INTO graph_runs (id, goal, spec_snapshot, compiled_graph, status) \
                 VALUES ('g2', 'g', '{}', ?1, 'running')",
                params![serde_json::to_string(&cg).unwrap()],
            )
            .unwrap();
            let (id, _) = create_node_run(
                &conn,
                "g2",
                "a",
                NodeType::Executor,
                1,
                Some("p1"),
                Some("w1"),
            )
            .unwrap();
            conn.execute(
                "UPDATE node_runs SET status='running', lease_expires_at=?1 WHERE id=?2",
                params![unix_now() - 60.0, id],
            )
            .unwrap();
        }

        let n = reconcile_all(&storage).unwrap();
        assert_eq!(n, 1, "仅 timed_out，无重试");
        assert!(
            {
                let conn = storage.conn();
                crate::graph::state::get_node_run_by_key(&conn, "g2", "a", 2)
                    .unwrap()
                    .is_none()
            },
            "不应创建 attempt 2"
        );
        let gstatus: String = {
            let conn = storage.conn();
            conn.query_row("SELECT status FROM graph_runs WHERE id='g2'", [], |r| {
                r.get(0)
            })
            .unwrap()
        };
        assert_eq!(gstatus, "failed", "不可重试超时 → 图 failed");
    }
}
