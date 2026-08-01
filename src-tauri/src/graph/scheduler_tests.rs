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
        conflict_pairs: vec![],
    };
    let items = tick(&conn, "g2", &compiled).unwrap();
    assert!(items.is_empty(), "spec 节点缺失时不应派发");
}

// ---- P1-1：并行派发 / 冲突感知 / Join 汇聚 / on_failure 触发 ----

fn setup_graph(
    storage: &Storage,
    yaml: &str,
    run_id: &str,
) -> crate::graph::compiler::CompiledGraph {
    let conn = storage.conn();
    let spec = crate::graph::spec::GraphSpec::parse(yaml).unwrap();
    let compiled = crate::graph::compiler::compile(&spec).compiled.unwrap();
    conn.execute(
        "INSERT INTO graph_runs (id, goal, spec_snapshot, compiled_graph, status) \
         VALUES (?1, 'g', '{}', '{}', 'ready')",
        rusqlite::params![run_id],
    )
    .unwrap();
    for n in &compiled.nodes {
        crate::graph::state::create_node_run(
            &conn,
            run_id,
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

fn mark_failed(storage: &Storage, run_id: &str, key: &str, failure_type: &str) {
    // 失败必须经 verifying（状态机 Dispatched→Running→Verifying→Failed）
    let conn = storage.conn();
    let mut run = crate::graph::state::get_node_run_by_key(&conn, run_id, key, 1)
        .unwrap()
        .unwrap();
    for (from, to) in [
        (NodeRunStatus::Dispatched, NodeRunStatus::Running),
        (NodeRunStatus::Running, NodeRunStatus::Verifying),
    ] {
        let out = crate::graph::state::transition(
            &conn,
            &run.id,
            run.version,
            from,
            to,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(out, crate::graph::state::TransitionOutcome::Applied);
        run = crate::graph::state::get_node_run(&conn, &run.id)
            .unwrap()
            .unwrap();
    }
    let out = crate::graph::state::transition(
        &conn,
        &run.id,
        run.version,
        NodeRunStatus::Verifying,
        NodeRunStatus::Failed,
        Some(failure_type),
        Some("boom"),
        None,
    )
    .unwrap();
    assert_eq!(out, crate::graph::state::TransitionOutcome::Applied);
}

fn mark_succeeded(storage: &Storage, run_id: &str, key: &str) {
    let conn = storage.conn();
    let run = crate::graph::state::get_node_run_by_key(&conn, run_id, key, 1)
        .unwrap()
        .unwrap();
    let out = crate::graph::state::transition(
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
    assert_eq!(out, crate::graph::state::TransitionOutcome::Applied);
    let run = crate::graph::state::get_node_run(&conn, &run.id)
        .unwrap()
        .unwrap();
    let out = crate::graph::state::transition(
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
    assert_eq!(out, crate::graph::state::TransitionOutcome::Applied);
    let run = crate::graph::state::get_node_run(&conn, &run.id)
        .unwrap()
        .unwrap();
    let out = crate::graph::state::transition(
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
    assert_eq!(out, crate::graph::state::TransitionOutcome::Applied);
}

#[test]
fn parallel_dispatch_two_non_conflicting_writers() {
    let storage = Storage::open_in_memory().unwrap();
    let yaml = r#"
version: 1
goal: "并行"
max_concurrency: 2
nodes:
  - id: a
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p1 }
    workspace: w1
    write_paths: [src/backend]
    acceptance: [{ type: path }]
    timeout_seconds: 300
  - id: b
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p1 }
    workspace: w2
    write_paths: [src/frontend]
    acceptance: [{ type: path }]
    timeout_seconds: 300
"#;
    let compiled = setup_graph(&storage, yaml, "g1");
    let conn = storage.conn();
    let items = tick(&conn, "g1", &compiled).unwrap();
    assert_eq!(items.len(), 2, "两个无冲突写节点应并行派发");
    let keys: Vec<&str> = items.iter().map(|i| i.node_key.as_str()).collect();
    assert!(keys.contains(&"a") && keys.contains(&"b"));
}

#[test]
fn conflicting_writers_serialized() {
    let storage = Storage::open_in_memory().unwrap();
    let yaml = r#"
version: 1
goal: "conflict serial"
max_concurrency: 2
nodes:
  - id: a
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p1 }
    workspace: w1
    write_paths: [src/backend]
    acceptance: [{ type: path }]
    timeout_seconds: 300
  - id: b
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p1 }
    workspace: w1
    write_paths: [src/backend/api.rs]
    acceptance: [{ type: path }]
    timeout_seconds: 300
"#;
    let compiled = setup_graph(&storage, yaml, "g2");
    assert!(compiled.conflict_pairs.len() >= 1, "compiler 应产出冲突对");
    let conn = storage.conn();
    let items = tick(&conn, "g2", &compiled).unwrap();
    assert_eq!(items.len(), 1, "冲突写节点同轮只派发一个");
}

#[test]
fn join_all_succeeded_gates_downstream() {
    let storage = Storage::open_in_memory().unwrap();
    let yaml = r#"
version: 1
goal: "join"
max_concurrency: 2
nodes:
  - id: a
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p1 }
    workspace: w1
    write_paths: [src/a]
    acceptance: [{ type: path }]
    timeout_seconds: 300
  - id: b
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p1 }
    workspace: w2
    write_paths: [src/b]
    acceptance: [{ type: path }]
    timeout_seconds: 300
  - id: j
    type: join
    dependencies: [a, b]
    join_policy: all_succeeded
    timeout_seconds: 60
  - id: c
    type: executor
    dependencies: [j]
    outputs: { schema: s }
    executor_requirements: { participant: p1 }
    workspace: w3
    write_paths: [src/c]
    acceptance: [{ type: path }]
    timeout_seconds: 300
"#;
    let compiled = setup_graph(&storage, yaml, "g3");
    {
        let conn = storage.conn();
        let items = tick(&conn, "g3", &compiled).unwrap();
        assert_eq!(items.len(), 2, "首轮 a、b 并行");
    }
    mark_succeeded(&storage, "g3", "a");
    mark_succeeded(&storage, "g3", "b");
    {
        let conn = storage.conn();
        let items = tick(&conn, "g3", &compiled).unwrap();
        assert_eq!(items.len(), 1, "join 汇聚后只派发 c");
        assert_eq!(items[0].node_key, "c");
    }
    let conn = storage.conn();
    let j = crate::graph::state::get_node_run_by_key(&conn, "g3", "j", 1)
        .unwrap()
        .unwrap();
    assert_eq!(j.status, NodeRunStatus::Succeeded, "join 汇聚后自动成功");
}

#[test]
fn on_failure_edge_triggers_repair_node() {
    let storage = Storage::open_in_memory().unwrap();
    let yaml = r#"
version: 1
goal: "repair trigger"
nodes:
  - id: a
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p1 }
    workspace: w1
    write_paths: [src/a]
    acceptance: [{ type: path }]
    timeout_seconds: 300
  - id: b
    type: executor
    on_failure: [a]
    outputs: { schema: s }
    executor_requirements: { participant: p1 }
    workspace: w2
    write_paths: [src/a]
    acceptance: [{ type: path }]
    timeout_seconds: 300
"#;
    let compiled = setup_graph(&storage, yaml, "g4");
    {
        let conn = storage.conn();
        let items = tick(&conn, "g4", &compiled).unwrap();
        assert_eq!(items.len(), 1, "首轮只派发 a");
    }
    mark_failed(&storage, "g4", "a", "execution_error");
    {
        let conn = storage.conn();
        let a_after = crate::graph::state::get_node_run_by_key(&conn, "g4", "a", 1)
            .unwrap()
            .unwrap();
        eprintln!(
            "[dbg] a status after fail: {:?} failure={:?}",
            a_after.status, a_after.failure_type
        );
        let b_before = crate::graph::state::get_node_run_by_key(&conn, "g4", "b", 1)
            .unwrap()
            .unwrap();
        eprintln!("[dbg] b status before round2: {:?}", b_before.status);
        let items = tick(&conn, "g4", &compiled).unwrap();
        eprintln!(
            "[dbg] round2 items: {:?}",
            items.iter().map(|i| i.node_key.clone()).collect::<Vec<_>>()
        );
        assert_eq!(items.len(), 1, "b 应被 on_failure 触发派发");
        assert_eq!(items[0].node_key, "b");
    }
    let conn = storage.conn();
    let b = crate::graph::state::get_node_run_by_key(&conn, "g4", "b", 1)
        .unwrap()
        .unwrap();
    assert_eq!(b.status, NodeRunStatus::Dispatched);
}
