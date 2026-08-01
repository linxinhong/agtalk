//! 节点结果验证与状态推进（M2，docs/design_graph.md §七/§八）。
//! 独立文件以控制 handlers/graph.rs 行数在红线内。

use crate::graph::compiler::CompiledGraph;
use crate::graph::scheduler::{tick, DispatchItem};
use crate::graph::state::{
    converge_graph_run, get_node_run, get_node_run_by_key, transition, NodeRunStatus,
};
use crate::proto::ServerMsg;
use crate::server::state::AppState;
use rusqlite::{params, OptionalExtension};

use super::graph::NodeBody;

/// M2 验证推进结果。
pub(crate) struct AdvanceOutcome {
    pub(crate) cg: CompiledGraph,
    pub(crate) items: Vec<DispatchItem>,
    pub(crate) node_status: String, // succeeded / failed / retrying
    pub(crate) message: String,
}

/// 验证 + 状态推进（design_graph.md §七 / §八）：
/// running → verifying →（验证）→ succeeded；验证失败 → failed，
/// 可重试（retry_policy.retryable 且未超 max_attempts）→ 新建 attempt（pending→tick）。
#[allow(clippy::result_large_err)]
pub(crate) fn verify_and_advance(
    state: &AppState,
    body: &NodeBody,
) -> Result<AdvanceOutcome, ServerMsg> {
    let now = unix_now();
    let (cg, items, node_status, message) = {
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
        // running → verifying
        transition(
            &conn,
            &run.id,
            run.version,
            NodeRunStatus::Running,
            NodeRunStatus::Verifying,
            None,
            None,
            None,
        )
        .map_err(graph_err)?;

        // claims 落库（M1，留证）
        for claim in &body.verification_claims {
            conn.execute(
                "INSERT INTO verifications (id, node_run_id, verifier_type, rule, expected, \
                 actual, status, completed_at) VALUES (?1,?2,'claim',?3,'0',?4,'recorded',?5)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    run.id,
                    claim.command,
                    claim.exit_code,
                    now
                ],
            )
            .map_err(sqlite_err)?;
        }

        // 读 compiled_graph + spec_node
        let compiled_json: String = conn
            .query_row(
                "SELECT compiled_graph FROM graph_runs WHERE id=?1",
                params![body.run_id],
                |r| r.get(0),
            )
            .map_err(sqlite_err)?;
        let cg: CompiledGraph = serde_json::from_str(&compiled_json).map_err(json_err)?;
        let spec_node = cg
            .nodes
            .iter()
            .find(|n| n.id == body.node_key)
            .ok_or_else(|| {
                err(
                    "graph_node_not_found",
                    format!("spec 无节点 {}", body.node_key),
                )
            })?;

        // M2 验证（路径 / artifact / schema，不 spawn）
        // worktree 根（symlink canonicalize 防护）；无 worktree（降级）传 None
        let ws_root = crate::graph::workspace::get_by_node(&conn, &body.run_id, &body.node_key)
            .ok()
            .flatten()
            .and_then(|ws| ws.path)
            .map(std::path::PathBuf::from);
        let v = crate::graph::verify::verify_node_result(
            ws_root.as_deref(),
            &body.changed_files,
            &spec_node.write_paths,
            &spec_node.forbidden_paths,
            &body.output_artifacts,
            &body.result,
        );
        for check in &v.checks {
            conn.execute(
                "INSERT INTO verifications (id, node_run_id, verifier_type, rule, expected, \
                 actual, status, completed_at) VALUES (?1,?2,?3,?4,'',?5,?6,?7)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    run.id,
                    check.verifier_type,
                    check.rule,
                    check.detail,
                    check.status,
                    now
                ],
            )
            .map_err(sqlite_err)?;
        }

        if v.passed {
            // ① 写节点先 commit worktree（进入 succeeded 前；失败 → 节点回滚 failed，不悬挂。
            //    修复 Tim 评审风险②：commit 失败不再让节点卡在 succeeded 且图不收敛）
            if !spec_node.write_paths.is_empty() {
                if let Ok(Some(ws)) =
                    crate::graph::workspace::get_by_node(&conn, &body.run_id, &body.node_key)
                {
                    let allowed: Vec<String> = body
                        .changed_files
                        .iter()
                        .filter(|f| {
                            spec_node
                                .write_paths
                                .iter()
                                .any(|w| crate::graph::paths::paths_overlap(w, f))
                        })
                        .cloned()
                        .collect();
                    if let Err(e) = crate::graph::workspace::commit_worktree(
                        &conn,
                        &ws,
                        &allowed,
                        &format!("graph {} {}", body.run_id, body.node_key),
                    ) {
                        // commit 失败 → verifying → failed（workspace_failure）
                        let run2 = get_node_run(&conn, &run.id)
                            .map_err(graph_err)?
                            .ok_or_else(|| err("graph_node_not_found", "节点不存在"))?;
                        let _ = transition(
                            &conn,
                            &run.id,
                            run2.version,
                            NodeRunStatus::Verifying,
                            NodeRunStatus::Failed,
                            Some("workspace_failure"),
                            Some(&e),
                            None,
                        )
                        .map_err(graph_err)?;
                        let _ = converge_graph_run(&conn, &body.run_id).map_err(graph_err)?;
                        return Ok(AdvanceOutcome {
                            cg,
                            items: Vec::new(),
                            node_status: "failed".into(),
                            message: format!("worktree commit 失败，节点回滚 failed: {e}"),
                        });
                    }
                }
            }
            // verifying → succeeded
            let run2 = get_node_run(&conn, &run.id)
                .map_err(graph_err)?
                .ok_or_else(|| err("graph_node_not_found", "节点不存在"))?;
            transition(
                &conn,
                &run.id,
                run2.version,
                NodeRunStatus::Verifying,
                NodeRunStatus::Succeeded,
                None,
                None,
                None,
            )
            .map_err(graph_err)?;
            // 产出物引用
            if !body.output_artifacts.is_empty() {
                conn.execute(
                    "UPDATE node_runs SET output_artifact_ids=?1 WHERE id=?2",
                    params![
                        serde_json::to_string(&body.output_artifacts).map_err(json_err)?,
                        run.id
                    ],
                )
                .map_err(sqlite_err)?;
            }
            // P1-3：修复节点（on_failure 声明）成功后，触发其上游失败节点重试（新 attempt）
            if !spec_node.on_failure.is_empty() {
                for f in &spec_node.on_failure {
                    let latest_attempt: Option<u32> = conn
                        .query_row(
                            "SELECT MAX(attempt) FROM node_runs WHERE graph_run_id=?1 AND node_key=?2",
                            params![body.run_id, f],
                            |r| r.get(0),
                        )
                        .optional()
                        .map_err(sqlite_err)?;
                    if let Some(att) = latest_attempt {
                        if let Ok(Some(f_run)) = get_node_run_by_key(&conn, &body.run_id, f, att) {
                            if matches!(
                                f_run.status,
                                NodeRunStatus::Failed | NodeRunStatus::TimedOut
                            ) {
                                let (_, created) = crate::graph::state::create_node_run(
                                    &conn,
                                    &body.run_id,
                                    f,
                                    crate::graph::spec::NodeType::from_str_name(&f_run.node_type)
                                        .unwrap_or(crate::graph::spec::NodeType::Executor),
                                    att + 1,
                                    f_run.participant_id.as_deref(),
                                    f_run.workspace_id.as_deref(),
                                )
                                .map_err(graph_err)?;
                                if created {
                                    crate::graph::events::append(
                                        &conn,
                                        &body.run_id,
                                        "node_ready",
                                        Some(f),
                                        &serde_json::json!({
                                            "attempt": att + 1,
                                            "retry_after_repair": body.node_key,
                                        }),
                                    )
                                    .map_err(graph_err)?;
                                }
                            }
                        }
                    }
                }
            }
            let _ = converge_graph_run(&conn, &body.run_id).map_err(graph_err)?;
            // P1-2：图 completed → merge workspaces 到集成分支
            let gstatus: String = conn
                .query_row(
                    "SELECT status FROM graph_runs WHERE id=?1",
                    params![body.run_id],
                    |r| r.get(0),
                )
                .map_err(sqlite_err)?;
            if gstatus == "completed" {
                let target: String = conn
                    .query_row(
                        "SELECT COALESCE(integration_target, 'main') FROM graph_runs WHERE id=?1",
                        params![body.run_id],
                        |r| r.get(0),
                    )
                    .map_err(sqlite_err)?;
                match crate::graph::workspace::merge_workspaces(&conn, &body.run_id, &target) {
                    Ok((merged, conflicts)) => {
                        // 冲突已由 merge_workspaces 发 merge_conflict 事件（含手动解决指引）
                        let _ = crate::graph::events::append(
                            &conn,
                            &body.run_id,
                            "graph_merged",
                            None,
                            &serde_json::json!({ "merged": merged, "conflicts": conflicts }),
                        )
                        .map_err(graph_err)?;
                    }
                    Err(e) => {
                        return Err(err("graph_workspace_merge_failed", e));
                    }
                }
            }
            let items = tick(&conn, &body.run_id, &cg).map_err(graph_err)?;
            (
                cg,
                items,
                "succeeded".to_string(),
                "节点结果已通过验证（路径/artifact/schema）".to_string(),
            )
        } else {
            // verifying → failed
            let failure_type = v
                .failure_type
                .unwrap_or_else(|| "contract_violation".to_string());
            let detail = v
                .checks
                .iter()
                .find(|c| c.status == "failed")
                .map(|c| c.detail.clone())
                .unwrap_or_else(|| "验证未通过".to_string());
            let run2 = get_node_run(&conn, &run.id)
                .map_err(graph_err)?
                .ok_or_else(|| err("graph_node_not_found", "节点不存在"))?;
            transition(
                &conn,
                &run.id,
                run2.version,
                NodeRunStatus::Verifying,
                NodeRunStatus::Failed,
                Some(&failure_type),
                Some(&detail),
                None,
            )
            .map_err(graph_err)?;
            let _ = converge_graph_run(&conn, &body.run_id).map_err(graph_err)?;

            // 重试判定（design_graph.md §八：临时执行错误/超时可重试，路径/契约违规不重试）
            let rp = &spec_node.retry_policy;
            let retryable = rp.retryable.iter().any(|f| f.as_str() == failure_type);
            // P1-3 Repair：图中有节点声明 on_failure 指向本节点（修复者）→ 失败不自动重试，
            // 进入修复流程（修复节点由 P1-1 的 on_failure 触发在 tick 中派发；修复成功后触发本节点重试）
            let has_repair = cg
                .nodes
                .iter()
                .any(|n| n.on_failure.iter().any(|f| f == &body.node_key));
            if has_repair {
                let items = tick(&conn, &body.run_id, &cg).map_err(graph_err)?;
                return Ok(AdvanceOutcome {
                    cg,
                    items,
                    node_status: "failed_repairing".into(),
                    message: format!("验证失败（{failure_type}），进入修复流程"),
                });
            }
            if run.attempt < rp.max_attempts && retryable {
                let next_attempt = run.attempt + 1;
                let (_, created) = crate::graph::state::create_node_run(
                    &conn,
                    &body.run_id,
                    &body.node_key,
                    spec_node.node_type,
                    next_attempt,
                    run.participant_id.as_deref(),
                    run.workspace_id.as_deref(),
                )
                .map_err(graph_err)?;
                if created {
                    crate::graph::events::append(
                        &conn,
                        &body.run_id,
                        "node_ready",
                        Some(&body.node_key),
                        &serde_json::json!({
                            "attempt": next_attempt,
                            "retry_of_failure": failure_type,
                        }),
                    )
                    .map_err(graph_err)?;
                    let items = tick(&conn, &body.run_id, &cg).map_err(graph_err)?;
                    return Ok(AdvanceOutcome {
                        cg,
                        items,
                        node_status: "retrying".into(),
                        message: format!("验证失败（{failure_type}），重试 attempt {next_attempt}"),
                    });
                }
            }
            (
                cg,
                Vec::new(),
                "failed".to_string(),
                format!("验证失败（{failure_type}）：{detail}"),
            )
        }
    };
    Ok(AdvanceOutcome {
        cg,
        items,
        node_status,
        message,
    })
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
