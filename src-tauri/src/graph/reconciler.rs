//! Reconciler（docs/design_graph.md §5.7 / M2 基础版）。
//!
//! daemon 启动时收敛运行现场：
//! 1. 对 running/paused 的图调用 converge_graph_run（终态推导）；
//! 2. lease 过期（lease_expires_at < now）且状态 active 的节点 → timed_out。
//!    避免重复执行/重复建 worktree（幂等键 UNIQUE + workspaces 先查）。

use crate::storage::Storage;
use rusqlite::{params, Connection};

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

    Ok(handled)
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
}
