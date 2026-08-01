//! 串行 Scheduler（docs/design_graph.md §5.5）。
//!
//! M1：一次派发最多 max_concurrency 个节点（串行默认 1）。
//! 每轮 tick：按 execution_order 找第一个可派发节点，检查依赖（on_success 上游全部
//! succeeded）、依赖失败则标 blocked，推进 pending→ready→leased→dispatched，
//! 返回 DispatchItem 交给端点层发消息（msg/send + graph_node_assignments）。

use std::collections::BTreeMap;

use rusqlite::Connection;

use super::compiler::CompiledGraph;
use super::spec::NodeType;
use super::state::{get_node_run, list_node_runs, transition, NodeRunRow, NodeRunStatus};

/// 待派发节点（端点层负责实际发消息 + 关联落库）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchItem {
    pub node_run_id: String,
    pub graph_run_id: String,
    pub node_key: String,
    pub attempt: u32,
    pub node_type: NodeType,
    pub participant_id: Option<String>,
    pub workspace_id: Option<String>,
    pub goal: String,
}

/// 每轮调度（design_graph.md §5.5 步骤 1-9）。串行：最多派发 max_concurrency 个。
pub fn tick(
    conn: &Connection,
    graph_run_id: &str,
    compiled: &CompiledGraph,
) -> Result<Vec<DispatchItem>, super::Error> {
    // 0. 依赖已失败的 pending 节点 → blocked（迭代至收敛：级联依赖需多轮）
    for _ in 0..compiled.nodes.len() {
        let runs = list_node_runs(conn, graph_run_id)?;
        let latest = build_latest(&runs);
        let mut any = false;
        for node in &compiled.nodes {
            let Some(run) = latest.get(node.id.as_str()).copied() else {
                continue;
            };
            if run.status != NodeRunStatus::Pending {
                continue;
            }
            let deps_blocked = node.dependencies.iter().any(|d| {
                latest
                    .get(d.as_str())
                    .map(|r| r.status.is_terminal() && r.status != NodeRunStatus::Succeeded)
                    .unwrap_or(false)
            });
            if deps_blocked {
                let _ = transition(
                    conn,
                    &run.id,
                    run.version,
                    NodeRunStatus::Pending,
                    NodeRunStatus::Blocked,
                    Some("dependency_failed"),
                    Some("上游依赖失败，节点被阻塞"),
                    None,
                )?;
                any = true;
            }
        }
        if !any {
            break;
        }
    }
    // 派发循环内每轮刷新依赖快照（控制节点可能在上一迭代推进成功）
    let mut dispatched = Vec::new();
    let mut dispatched_keys: Vec<&str> = Vec::new();
    for node in &compiled.execution_order {
        if dispatched.len() as u32 >= compiled.max_concurrency {
            break;
        }
        // 每轮刷新依赖快照：控制节点（join/gate）可能在上一迭代已推进成功
        let runs = list_node_runs(conn, graph_run_id)?;
        let latest = build_latest(&runs);
        let Some(spec_node) = compiled.nodes.iter().find(|n| n.id == *node) else {
            continue;
        };
        let Some(run) = latest.get(node.as_str()) else {
            continue;
        };
        // 冲突感知（P1-1）：与本轮已派发节点有写冲突（write_paths/共享契约/同 workspace）→ 跳过等下一轮
        let conflicts_with_dispatched = dispatched_keys.iter().any(|d| {
            compiled
                .conflict_pairs
                .iter()
                .any(|(a, b)| (a == *d && b == node) || (a == node && b == *d))
        });
        if conflicts_with_dispatched {
            continue;
        }
        let Some(item) = try_dispatch(conn, run, spec_node, &latest, &compiled.goal)? else {
            continue;
        };
        dispatched.push(item);
        dispatched_keys.push(node.as_str());
    }
    Ok(dispatched)
}

/// 组装 node_key → 最新 attempt run 的索引。
fn build_latest(runs: &[NodeRunRow]) -> BTreeMap<&str, &NodeRunRow> {
    let mut latest: BTreeMap<&str, &NodeRunRow> = BTreeMap::new();
    for run in runs {
        let entry = latest.entry(run.node_key.as_str()).or_insert(run);
        if run.attempt > entry.attempt {
            *entry = run;
        }
    }
    latest
}

/// 尝试推进单个节点到 dispatched；返回 None = 暂不可派发。
fn try_dispatch(
    conn: &Connection,
    run: &NodeRunRow,
    spec_node: &super::spec::NodeSpec,
    latest: &BTreeMap<&str, &NodeRunRow>,
    goal: &str,
) -> Result<Option<DispatchItem>, super::Error> {
    // Join/Gate：无执行体控制节点（P1-1），汇聚/分叉后直接成功，不派发消息
    if matches!(
        spec_node.node_type,
        super::spec::NodeType::Join | super::spec::NodeType::Gate
    ) {
        return try_control_node(conn, run, spec_node, latest);
    }

    // on_failure 触发（P1-1）：on_failure 上游失败 → 本节点被激活（如 Repair 场景）
    let on_failure_triggered = spec_node.on_failure.iter().any(|f| {
        latest
            .get(f.as_str())
            .map(|r| r.status.is_terminal() && r.status != NodeRunStatus::Succeeded)
            .unwrap_or(false)
    });
    let deps_succeeded = spec_node.dependencies.iter().all(|d| {
        latest
            .get(d.as_str())
            .map(|r| r.status == NodeRunStatus::Succeeded)
            .unwrap_or(false)
    });
    let deps_blocked = spec_node.dependencies.iter().any(|d| {
        latest
            .get(d.as_str())
            .map(|r| r.status.is_terminal() && r.status != NodeRunStatus::Succeeded)
            .unwrap_or(false)
    });

    match run.status {
        NodeRunStatus::Pending => {
            // 纯 on_failure 分支（无 on_success 依赖，仅失败触发）：
            // 等待 on_failure 上游失败 → 激活；上游全部成功 → 分支未触发，标 blocked。
            if !spec_node.on_failure.is_empty() && spec_node.dependencies.is_empty() {
                if on_failure_triggered {
                    return advance_to_dispatched(conn, run, goal);
                }
                let on_failure_succeeded = spec_node.on_failure.iter().all(|f| {
                    latest
                        .get(f.as_str())
                        .map(|r| r.status == NodeRunStatus::Succeeded)
                        .unwrap_or(false)
                });
                if on_failure_succeeded {
                    let _ = transition(
                        conn,
                        &run.id,
                        run.version,
                        NodeRunStatus::Pending,
                        NodeRunStatus::Blocked,
                        Some("condition_not_met"),
                        Some("on_failure 触发条件未满足（上游成功），分支不执行"),
                        None,
                    )?;
                }
                return Ok(None);
            }
            if deps_blocked && !on_failure_triggered {
                // 依赖已失败（且无 on_failure 触发）→ 标 blocked（不重试、不派发）
                let _ = transition(
                    conn,
                    &run.id,
                    run.version,
                    NodeRunStatus::Pending,
                    NodeRunStatus::Blocked,
                    Some("dependency_failed"),
                    Some("上游依赖失败，节点被阻塞"),
                    None,
                )?;
                return Ok(None);
            }
            if !deps_succeeded && !on_failure_triggered {
                return Ok(None);
            }
            advance_to_dispatched(conn, run, goal)
        }
        NodeRunStatus::Ready => advance_to_dispatched(conn, run, goal),
        NodeRunStatus::Leased => finish_dispatch(run, goal),
        _ => Ok(None),
    }
}

/// Join/Gate 控制节点：依赖满足 → 逐级推进到 succeeded（无执行体，不产生 DispatchItem）。
fn try_control_node(
    conn: &Connection,
    run: &NodeRunRow,
    spec_node: &super::spec::NodeSpec,
    latest: &BTreeMap<&str, &NodeRunRow>,
) -> Result<Option<DispatchItem>, super::Error> {
    use super::spec::JoinPolicy;
    // 汇聚条件（P1-1）：Join 按 join_policy；Gate 要求 on_success 依赖全成功
    let proceed = match spec_node.node_type {
        super::spec::NodeType::Join => match spec_node.join_policy {
            Some(JoinPolicy::AllSucceeded) => spec_node.dependencies.iter().all(|d| {
                latest
                    .get(d.as_str())
                    .map(|r| r.status == NodeRunStatus::Succeeded)
                    .unwrap_or(false)
            }),
            Some(JoinPolicy::AllTerminal) => spec_node.dependencies.iter().all(|d| {
                latest
                    .get(d.as_str())
                    .map(|r| r.status.is_terminal())
                    .unwrap_or(false)
            }),
            None => false,
        },
        super::spec::NodeType::Gate => spec_node.dependencies.iter().all(|d| {
            latest
                .get(d.as_str())
                .map(|r| r.status == NodeRunStatus::Succeeded)
                .unwrap_or(false)
        }),
        _ => false,
    };
    if !proceed {
        // 依赖失败（非 all_terminal 语义）→ blocked
        let deps_failed = spec_node.dependencies.iter().any(|d| {
            latest
                .get(d.as_str())
                .map(|r| r.status.is_terminal() && r.status != NodeRunStatus::Succeeded)
                .unwrap_or(false)
        });
        if deps_failed && run.status == NodeRunStatus::Pending {
            let _ = transition(
                conn,
                &run.id,
                run.version,
                NodeRunStatus::Pending,
                NodeRunStatus::Blocked,
                Some("dependency_failed"),
                Some("上游依赖失败，控制节点被阻塞"),
                None,
            )?;
        }
        return Ok(None);
    }

    // 推进：pending/ready/leased → dispatched → succeeded（派发即完成）
    let chain = [
        NodeRunStatus::Pending,
        NodeRunStatus::Ready,
        NodeRunStatus::Leased,
        NodeRunStatus::Dispatched,
    ];
    let start_idx = chain
        .iter()
        .position(|s| *s == run.status)
        .unwrap_or(chain.len() - 1);
    let mut current = run.clone();
    for to in &chain[start_idx..] {
        if *to == current.status {
            continue;
        }
        let out = transition(
            conn,
            &current.id,
            current.version,
            current.status,
            *to,
            None,
            None,
            None,
        )?;
        if out != super::state::TransitionOutcome::Applied {
            return Ok(None);
        }
        let Some(r2) = get_node_run(conn, &current.id)? else {
            return Ok(None);
        };
        current = r2;
    }
    let out = transition(
        conn,
        &current.id,
        current.version,
        NodeRunStatus::Dispatched,
        NodeRunStatus::Succeeded,
        None,
        None,
        None,
    )?;
    if out != super::state::TransitionOutcome::Applied {
        return Ok(None);
    }
    Ok(None) // 控制节点不派发消息
}

/// pending/ready → leased → dispatched（乐观锁逐级推进；版本冲突则放弃本轮）。
fn advance_to_dispatched(
    conn: &Connection,
    run: &NodeRunRow,
    goal: &str,
) -> Result<Option<DispatchItem>, super::Error> {
    let from = if run.status == NodeRunStatus::Pending {
        NodeRunStatus::Pending
    } else {
        NodeRunStatus::Ready
    };
    let out = transition(
        conn,
        &run.id,
        run.version,
        from,
        NodeRunStatus::Ready,
        None,
        None,
        None,
    )?;
    if out != super::state::TransitionOutcome::Applied {
        return Ok(None);
    }
    let run2 = match get_node_run(conn, &run.id)? {
        Some(r) => r,
        None => return Ok(None),
    };
    let out = transition(
        conn,
        &run.id,
        run2.version,
        NodeRunStatus::Ready,
        NodeRunStatus::Leased,
        None,
        None,
        None,
    )?;
    if out != super::state::TransitionOutcome::Applied {
        return Ok(None);
    }
    let run3 = match get_node_run(conn, &run.id)? {
        Some(r) => r,
        None => return Ok(None),
    };
    let out = transition(
        conn,
        &run.id,
        run3.version,
        NodeRunStatus::Leased,
        NodeRunStatus::Dispatched,
        None,
        None,
        None,
    )?;
    if out != super::state::TransitionOutcome::Applied {
        return Ok(None);
    }
    let final_run = match get_node_run(conn, &run.id)? {
        Some(r) => r,
        None => return Ok(None),
    };
    finish_dispatch(&final_run, goal)
}

/// 组装 DispatchItem。
fn finish_dispatch(run: &NodeRunRow, goal: &str) -> Result<Option<DispatchItem>, super::Error> {
    let node_type =
        super::spec::NodeType::from_str_name(&run.node_type).unwrap_or(NodeType::Executor);
    Ok(Some(DispatchItem {
        node_run_id: run.id.clone(),
        graph_run_id: run.graph_run_id.clone(),
        node_key: run.node_key.clone(),
        attempt: run.attempt,
        node_type,
        participant_id: run.participant_id.clone(),
        workspace_id: run.workspace_id.clone(),
        goal: goal.to_string(),
    }))
}

#[cfg(test)]
#[path = "scheduler_tests.rs"]
mod tests;
