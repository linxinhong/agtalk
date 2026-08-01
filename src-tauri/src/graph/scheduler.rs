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
    // 刷新 latest（blocked 已写入）
    let runs = list_node_runs(conn, graph_run_id)?;
    let latest = build_latest(&runs);

    let mut dispatched = Vec::new();
    for node in &compiled.execution_order {
        if dispatched.len() as u32 >= compiled.max_concurrency {
            break;
        }
        let Some(spec_node) = compiled.nodes.iter().find(|n| n.id == *node) else {
            continue;
        };
        let Some(run) = latest.get(node.as_str()) else {
            continue;
        };
        let Some(item) = try_dispatch(conn, run, spec_node, &latest, &compiled.goal)? else {
            continue;
        };
        dispatched.push(item);
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
            if deps_blocked {
                // 依赖已失败 → 本节点不可能成功，标 blocked（不重试、不派发）
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
            if !deps_succeeded {
                return Ok(None);
            }
            advance_to_dispatched(conn, run, goal)
        }
        NodeRunStatus::Ready => advance_to_dispatched(conn, run, goal),
        NodeRunStatus::Leased => finish_dispatch(run, goal),
        _ => Ok(None),
    }
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
mod tests {
    use super::*;
    use crate::graph::spec::GraphSpec;
    use crate::storage::Storage;

    fn setup(conn: &Connection) -> CompiledGraph {
        let yaml = r#"
version: 1
goal: "后端实现与验证"
max_concurrency: 1
nodes:
  - id: impl-backend
    type: executor
    outputs: { schema: source-diff }
    executor_requirements: { participant: backend-agent }
    workspace: w1
    write_paths: [src/backend]
    acceptance: [{ type: path }]
    timeout_seconds: 300
  - id: test-backend
    type: deterministic
    dependencies: [impl-backend]
    outputs: { schema: test-report }
    acceptance: [{ type: command }]
    timeout_seconds: 120
  - id: verify
    type: deterministic
    dependencies: [test-backend]
    outputs: { schema: verification-report }
    acceptance: [{ type: artifact }]
    timeout_seconds: 60
"#;
        let spec = GraphSpec::parse(yaml).unwrap();
        let compiled = crate::graph::compiler::compile(&spec).compiled.unwrap();
        conn.execute(
            "INSERT INTO graph_runs (id, goal, spec_snapshot, compiled_graph, status) \
             VALUES ('g1', 'goal', '{}', '{}', 'ready')",
            [],
        )
        .unwrap();
        // 预建所有节点 attempt=1
        for n in &compiled.nodes {
            crate::graph::state::create_node_run(
                conn,
                "g1",
                &n.id,
                n.node_type,
                1,
                n.executor_requirements.participant.as_deref(),
                n.workspace.as_deref(),
            )
            .unwrap();
        }
        compiled
    }

    #[test]
    fn serial_dispatch_one_at_a_time() {
        let storage = Storage::open_in_memory().unwrap();
        let conn = storage.conn();
        let compiled = setup(&conn);

        // 第一轮：只派发 impl-backend（串行 max_concurrency=1）
        let items = tick(&conn, "g1", &compiled).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].node_key, "impl-backend");
        assert_eq!(items[0].participant_id.as_deref(), Some("backend-agent"));

        // 未完成上游 → 第二轮无派发
        assert!(tick(&conn, "g1", &compiled).unwrap().is_empty());

        // 标记 impl-backend succeeded
        let run = crate::graph::state::get_node_run_by_key(&conn, "g1", "impl-backend", 1)
            .unwrap()
            .unwrap();
        let r = crate::graph::state::transition(
            &conn,
            &run.id,
            run.version,
            NodeRunStatus::Dispatched,
            NodeRunStatus::Running,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(r, crate::graph::state::TransitionOutcome::Applied);
        let run = crate::graph::state::get_node_run(&conn, &run.id)
            .unwrap()
            .unwrap();
        let r = crate::graph::state::transition(
            &conn,
            &run.id,
            run.version,
            NodeRunStatus::Running,
            NodeRunStatus::Verifying,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(r, crate::graph::state::TransitionOutcome::Applied);
        let run = crate::graph::state::get_node_run(&conn, &run.id)
            .unwrap()
            .unwrap();
        let r = crate::graph::state::transition(
            &conn,
            &run.id,
            run.version,
            NodeRunStatus::Verifying,
            NodeRunStatus::Succeeded,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(r, crate::graph::state::TransitionOutcome::Applied);

        // 第三轮：派发 test-backend
        let items = tick(&conn, "g1", &compiled).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].node_key, "test-backend");
    }

    #[test]
    fn dependency_failure_blocks_downstream() {
        let storage = Storage::open_in_memory().unwrap();
        let conn = storage.conn();
        let compiled = setup(&conn);

        let items = tick(&conn, "g1", &compiled).unwrap();
        assert_eq!(items[0].node_key, "impl-backend");
        // impl-backend failed（完整链：dispatched→running→verifying→failed）
        let run = crate::graph::state::get_node_run_by_key(&conn, "g1", "impl-backend", 1)
            .unwrap()
            .unwrap();
        let r = crate::graph::state::transition(
            &conn,
            &run.id,
            run.version,
            NodeRunStatus::Dispatched,
            NodeRunStatus::Running,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(r, crate::graph::state::TransitionOutcome::Applied);
        let run = crate::graph::state::get_node_run(&conn, &run.id)
            .unwrap()
            .unwrap();
        let r = crate::graph::state::transition(
            &conn,
            &run.id,
            run.version,
            NodeRunStatus::Running,
            NodeRunStatus::Verifying,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(r, crate::graph::state::TransitionOutcome::Applied);
        let run = crate::graph::state::get_node_run(&conn, &run.id)
            .unwrap()
            .unwrap();
        let r = crate::graph::state::transition(
            &conn,
            &run.id,
            run.version,
            NodeRunStatus::Verifying,
            NodeRunStatus::Failed,
            Some("execution_error"),
            Some("boom"),
            None,
        )
        .unwrap();
        assert_eq!(r, crate::graph::state::TransitionOutcome::Applied);

        // 下游 test-backend / verify 被标 blocked
        let items = tick(&conn, "g1", &compiled).unwrap();
        assert!(items.is_empty());
        let tb = crate::graph::state::get_node_run_by_key(&conn, "g1", "test-backend", 1)
            .unwrap()
            .unwrap();
        assert_eq!(
            tb.status,
            NodeRunStatus::Blocked,
            "依赖失败后下游应 blocked"
        );
        let vf = crate::graph::state::get_node_run_by_key(&conn, "g1", "verify", 1)
            .unwrap()
            .unwrap();
        assert_eq!(vf.status, NodeRunStatus::Blocked);
    }

    #[test]
    fn illegal_dispatch_skipped() {
        // pending 节点依赖不存在（ghost）→ 永不派发且不崩溃
        let storage = Storage::open_in_memory().unwrap();
        let conn = storage.conn();
        conn.execute(
            "INSERT INTO graph_runs (id, goal, spec_snapshot, compiled_graph, status) \
             VALUES ('g2', 'goal', '{}', '{}', 'ready')",
            [],
        )
        .unwrap();
        crate::graph::state::create_node_run(
            &conn,
            "g2",
            "a",
            NodeType::Executor,
            1,
            Some("p1"),
            Some("w1"),
        )
        .unwrap();

        // 手工构造 minimal compiled（只有一个节点 a，依赖 ghost —— 正常 compile 会拒，这里直接测 tick 容错）
        let compiled = CompiledGraph {
            goal: "g".into(),
            repository: None,
            base_revision: None,
            integration_target: None,
            max_concurrency: 1,
            nodes: vec![],
            edges: vec![],
            execution_order: vec!["a".into()],
            parallel_groups: vec![],
            required_approvals: vec![],
            resource_conflicts: vec![],
        };
        let items = tick(&conn, "g2", &compiled).unwrap();
        assert!(items.is_empty(), "spec 节点缺失时不应派发");
    }
}
