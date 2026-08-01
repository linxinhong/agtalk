//! GraphEvent 追加式事件（docs/design_graph.md §5.2 graph_events 表）。
//!
//! 纪律：**先落库后推送**（design.md §11 红线 6）。本模块只负责持久化与读取，
//! SSE 推送在端点层（server/handlers/graph.rs）基于本模块的数据广播。

use rusqlite::{params, Connection};
use serde_json::Value;

/// 事件类型清单（design_graph.md §7）。
pub const EVENT_TYPES: &[&str] = &[
    "graph_created",
    "graph_validated",
    "graph_started",
    "graph_paused",
    "graph_resumed",
    "graph_completed",
    "graph_failed",
    "graph_cancelled",
    "node_ready",
    "node_leased",
    "node_dispatched",
    "node_started",
    "node_progress",
    "node_verifying",
    "node_succeeded",
    "node_failed",
    "node_blocked",
    "node_timed_out",
    "node_waiting_approval",
    "workspace_created",
    "workspace_dirty",
    "workspace_committed",
    "workspace_merged",
    "workspace_released",
    "artifact_created",
    "verification_completed",
    "assignment_dispatched",
    "assignment_result",
];

/// GraphEvent 行（读模型）。
#[derive(Debug, Clone, PartialEq)]
pub struct GraphEvent {
    pub id: i64,
    pub graph_run_id: String,
    pub event_type: String,
    pub node_key: Option<String>,
    pub payload: Value,
    pub created_at: f64,
}

/// 追加事件（先落库）。event_type 不做白名单强校验（M1 起由调用方保证合法）；
/// id 为全局单调自增（SSE Last-Event-ID 重放锚点）。
pub fn append(
    conn: &Connection,
    graph_run_id: &str,
    event_type: &str,
    node_key: Option<&str>,
    payload: &Value,
) -> Result<i64, super::Error> {
    conn.execute(
        "INSERT INTO graph_events (graph_run_id, event_type, node_key, payload, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            graph_run_id,
            event_type,
            node_key,
            serde_json::to_string(payload).map_err(super::Error::Json)?,
            unix_now(),
        ],
    )
    .map_err(super::Error::Sqlite)?;
    Ok(conn.last_insert_rowid())
}

/// 读取某图的事件（可选从 after_id 之后、limit 条）。
pub fn list_after(
    conn: &Connection,
    graph_run_id: &str,
    after_id: Option<i64>,
    limit: u32,
) -> Result<Vec<GraphEvent>, super::Error> {
    let limit = limit.clamp(1, 1000);
    let mut stmt = conn
        .prepare(
            "SELECT id, graph_run_id, event_type, node_key, payload, created_at \
             FROM graph_events \
             WHERE graph_run_id=?1 AND (?2 IS NULL OR id > ?2) \
             ORDER BY id LIMIT ?3",
        )
        .map_err(super::Error::Sqlite)?;
    let rows = stmt
        .query_map(params![graph_run_id, after_id, limit], |r| {
            Ok(GraphEvent {
                id: r.get(0)?,
                graph_run_id: r.get(1)?,
                event_type: r.get(2)?,
                node_key: r.get(3)?,
                payload: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or(Value::Null),
                created_at: r.get(5)?,
            })
        })
        .map_err(super::Error::Sqlite)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(super::Error::Sqlite)?;
    Ok(rows)
}

/// 最新事件 id（SSE 初次连接时作为起始游标，不重放历史）。
pub fn max_event_id(conn: &Connection, graph_run_id: &str) -> Result<i64, super::Error> {
    conn.query_row(
        "SELECT COALESCE(MAX(id), 0) FROM graph_events WHERE graph_run_id=?1",
        params![graph_run_id],
        |r| r.get(0),
    )
    .map_err(super::Error::Sqlite)
}

fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;

    #[test]
    fn append_and_list_with_cursor() {
        let storage = Storage::open_in_memory().unwrap();
        let conn = storage.conn();
        conn.execute(
            "INSERT INTO graph_runs (id, goal, spec_snapshot, compiled_graph) \
             VALUES ('g1', 'g', '{}', '{}')",
            [],
        )
        .unwrap();

        let e1 = append(
            &conn,
            "g1",
            "graph_created",
            None,
            &serde_json::json!({"a": 1}),
        )
        .unwrap();
        let e2 = append(
            &conn,
            "g1",
            "node_ready",
            Some("n1"),
            &serde_json::json!({}),
        )
        .unwrap();
        assert!(e2 > e1, "事件 id 应单调递增");

        let all = list_after(&conn, "g1", None, 100).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].event_type, "graph_created");
        assert_eq!(all[1].node_key.as_deref(), Some("n1"));
        assert_eq!(all[1].payload, serde_json::json!({}));

        let after_e1 = list_after(&conn, "g1", Some(e1), 100).unwrap();
        assert_eq!(after_e1.len(), 1);
        assert_eq!(after_e1[0].id, e2);

        assert_eq!(max_event_id(&conn, "g1").unwrap(), e2);
        // 不存在的图 → 0
        assert_eq!(max_event_id(&conn, "g-ghost").unwrap(), 0);
    }
}
