//! 跨节点 artifact 传递（Tim 评审 M2：artifact store + input_summaries）。
//!
//! 节点 succeeded 时把产出物复制到 daemon 统一 store（`<config>/artifacts/<run>/<node>/`），
//! 下游派发时携带 input_artifacts（store 路径，只读引用）与 input_summaries（上游 result 摘要）。
//! 取舍：直接引用 store 而非复制到下游 worktree——产物是只读输入（隔离保护的是写冲突），
//! store GC 随 run 删除（run 活着 store 就在），满足隔离且简化。
//! 安全：路径禁 `..` 逃逸；单文件大小上限；只复制 outputs.artifacts 声明的产物。

use crate::graph::compiler::CompiledGraph;
use crate::graph::dto::GraphArtifactRef;
use crate::storage::Storage;
use rusqlite::{params, Connection};

/// 单文件大小上限（10MB）：build 产物级大文件不默认传（Tim 评审补强 c）。
pub const MAX_ARTIFACT_BYTES: u64 = 10 * 1024 * 1024;

/// 复制产物到 store 并注册 artifacts 表（验证通过后调用）。
pub fn store_run_artifacts(
    conn: &Connection,
    run_id: &str,
    node_key: &str,
    node_run_id: &str,
    artifacts: &[GraphArtifactRef],
) -> Result<(), String> {
    if run_id.contains("..") || node_key.contains("..") {
        return Err("run_id/node_key 含 '..'，拒绝写入 artifact store".into());
    }
    if artifacts.is_empty() {
        return Ok(());
    }
    let root = crate::paths::artifact_store_root().map_err(|e| e.to_string())?;
    let dir = root.join(run_id).join(node_key);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 artifact store 失败: {e}"))?;

    for a in artifacts {
        let Some(src) = local_path(&a.uri) else {
            continue; // 非本地 uri（http 等）不落 store
        };
        if !src.exists() {
            continue;
        }
        if src.metadata().map_err(|e| e.to_string())?.len() > MAX_ARTIFACT_BYTES {
            continue; // 超过上限跳过（大文件不默认传）
        }
        let Some(name) = src.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.contains("..") || name.contains('/') {
            continue; // 禁路径逃逸
        }
        let dst = dir.join(name);
        if let Err(e) = std::fs::copy(src, &dst) {
            eprintln!("[graph] artifact 复制失败 {:?} -> {:?}: {e}", src, dst);
            continue;
        }
        conn.execute(
            "INSERT INTO artifacts (id, graph_run_id, producer_node_run_id, artifact_type, schema_version, uri, checksum, metadata) VALUES (?1,?2,?3,?4,?5,?6,?7,'{}')", params![
                uuid::Uuid::new_v4().to_string(),
                run_id,
                node_run_id,
                a.artifact_type,
                a.schema_version,
                dst.to_string_lossy().into_owned(),
                a.checksum,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 收集本节点的上游输入：input_artifacts（上游 succeeded 产物 store 路径）+ input_summaries（上游 result 摘要）。
pub fn collect_upstream_inputs(
    storage: &Storage,
    compiled: &CompiledGraph,
    run_id: &str,
    node_key: &str,
) -> Result<(Vec<serde_json::Value>, Vec<serde_json::Value>), String> {
    // 上游 = 本节点依赖的节点（Edge.from=依赖者，to=被依赖者 → 本节点的前置 = from==node 的 to）
    let upstream: Vec<String> = compiled
        .edges
        .iter()
        .filter(|e| e.from == node_key)
        .map(|e| e.to.clone())
        .collect();
    let mut artifacts = Vec::new();
    let mut summaries = Vec::new();
    if upstream.is_empty() {
        return Ok((artifacts, summaries));
    }
    let conn = storage.conn();
    for up in &upstream {
        // 上游最新 attempt 且 succeeded
        let row: Option<(String, String, String)> = conn
            .query_row(
                "SELECT id, result, output_artifact_ids FROM node_runs WHERE graph_run_id=?1 AND node_key=?2 AND status='succeeded' ORDER BY attempt DESC LIMIT 1",
                params![run_id, up],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok();
        let Some((node_run_id, result, artifact_ids_json)) = row else {
            continue;
        };
        if !result.trim().is_empty() {
            summaries.push(serde_json::json!({ "node": up, "summary": result }));
        }
        if let Ok(ids) = serde_json::from_str::<Vec<GraphArtifactRef>>(&artifact_ids_json) {
            for a in ids {
                let uri = store_uri_for(&conn, &node_run_id, &a.uri)?;
                artifacts.push(serde_json::json!({
                    "node": up,
                    "artifact_type": a.artifact_type,
                    "schema_version": a.schema_version,
                    "uri": uri,
                    "checksum": a.checksum,
                }));
            }
        }
    }
    Ok((artifacts, summaries))
}

/// 产物在 store 的路径（artifacts 表按 producer_node_run_id 查）；无记录返回原 uri。
fn store_uri_for(
    conn: &Connection,
    producer_node_run_id: &str,
    original_uri: &str,
) -> Result<String, String> {
    let uri: Option<String> = conn
        .query_row(
            "SELECT uri FROM artifacts WHERE producer_node_run_id=?1 LIMIT 1",
            params![producer_node_run_id],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    Ok(uri.unwrap_or_else(|| original_uri.to_string()))
}

fn local_path(uri: &str) -> Option<&std::path::Path> {
    let p = std::path::Path::new(uri);
    if p.starts_with("file://") {
        uri.strip_prefix("file://").map(std::path::Path::new)
    } else if p.is_absolute() {
        Some(p)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_dotdot_escape() {
        let storage = Storage::open_in_memory().unwrap();
        let conn = storage.conn();
        let r = store_run_artifacts(&conn, "run/../evil", "node", "nid", &[]);
        assert!(r.is_err(), "'..' 应被拒绝");
    }

    #[test]
    fn stores_and_collects() {
        let cfg_dir = tempfile::tempdir().unwrap();
        std::env::set_var("AGTALK_CONFIG_DIR", cfg_dir.path());
        let storage = Storage::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("out.json");
        std::fs::write(&f, r#"{"ok":true}"#).unwrap();
        let artifact = GraphArtifactRef {
            artifact_type: "source-diff".into(),
            schema_version: "v1".into(),
            uri: f.to_string_lossy().into_owned(),
            checksum: "sha256:abc".into(),
        };
        {
            let conn = storage.conn();
            conn.execute(
                "INSERT INTO graph_runs (id, goal, spec_snapshot, compiled_graph, status) VALUES ('g', 'g', '{}', '{}', 'ready')",
                [],
            )
            .unwrap();
            let (nid, _) = crate::graph::state::create_node_run(
                &conn,
                "g",
                "up",
                crate::graph::spec::NodeType::Executor,
                1,
                Some("p"),
                None,
            )
            .unwrap();
            store_run_artifacts(&conn, "g", "up", &nid, &[artifact]).unwrap();
            // 更新 output_artifact_ids（模拟 apply_result）
            let stored = vec![GraphArtifactRef {
                artifact_type: "source-diff".into(),
                schema_version: "v1".into(),
                uri: f.to_string_lossy().into_owned(),
                checksum: "sha256:abc".into(),
            }];
            conn.execute(
                "UPDATE node_runs SET status='succeeded', result='完成', output_artifact_ids=?1 WHERE id=?2",
                params![serde_json::to_string(&stored).unwrap(), nid],
            )
            .unwrap();
        }
        // 下游节点收集上游输入
        let compiled = CompiledGraph {
            goal: "g".into(),
            repository: None,
            base_revision: None,
            integration_target: None,
            max_concurrency: 2,
            nodes: vec![],
            edges: vec![crate::graph::compiler::Edge {
                from: "down".into(),
                to: "up".into(),
                trigger: crate::graph::compiler::Trigger::OnSuccess,
            }],
            execution_order: vec!["up".into(), "down".into()],
            parallel_groups: vec![],
            required_approvals: vec![],
            resource_conflicts: vec![],
            conflict_pairs: vec![],
        };
        let (artifacts, summaries) =
            collect_upstream_inputs(&storage, &compiled, "g", "down").unwrap();
        {
            let conn = storage.conn();
            let r: Option<(String, String)> = conn
                .query_row(
                    "SELECT status, result FROM node_runs WHERE node_key='up'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok();
            eprintln!("UP ROW: {r:?}");
        }
        assert_eq!(summaries.len(), 1, "应有上游 result 摘要");
        assert_eq!(summaries[0]["summary"], "完成");
        assert_eq!(artifacts.len(), 1, "应有上游产物");
        assert!(
            artifacts[0]["uri"]
                .as_str()
                .unwrap()
                .contains("artifacts/g/up/"),
            "产物应指向 store 路径: {}",
            artifacts[0]["uri"]
        );
    }
}
