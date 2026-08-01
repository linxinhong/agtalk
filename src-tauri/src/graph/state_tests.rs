//! NodeRun / GraphRun 状态机测试（就近原则；state.rs 实现保持行数红线内）。

use crate::graph::spec::NodeType;
use crate::graph::state::{
    can_transition, converge_graph_run, create_node_run, get_node_run, get_node_run_by_key,
    transition, GraphRunStatus, NodeRunStatus, TransitionOutcome,
};
use crate::storage::Storage;
use rusqlite::Connection;

fn mem() -> Storage {
    Storage::open_in_memory().unwrap()
}

fn seed_run(conn: &Connection) -> String {
    conn.execute(
        "INSERT INTO graph_runs (id, goal, spec_snapshot, compiled_graph, status) \
         VALUES ('g1', 'goal', '{}', '{}', 'ready')",
        [],
    )
    .unwrap();
    let (id, created) = create_node_run(
        conn,
        "g1",
        "a",
        NodeType::Executor,
        1,
        Some("p1"),
        Some("w1"),
    )
    .unwrap();
    assert!(created);
    id
}

#[test]
fn terminal_states_are_terminal() {
    assert!(NodeRunStatus::Succeeded.is_terminal());
    assert!(!NodeRunStatus::Running.is_terminal());
    assert!(NodeRunStatus::Running.is_active());
    assert!(!NodeRunStatus::Pending.is_active());
}

#[test]
fn migration_table_known_transitions() {
    use NodeRunStatus::*;
    assert!(can_transition(Pending, Ready));
    assert!(can_transition(Running, Verifying));
    assert!(can_transition(Verifying, Succeeded));
    assert!(can_transition(WaitingApproval, Running));
    assert!(!can_transition(Pending, Succeeded), "不能跳过中间态");
    assert!(!can_transition(Succeeded, Failed), "终态不迁移");
}

#[test]
fn legal_transition_applies_with_optimistic_lock() {
    let storage = mem();
    let conn = storage.conn();
    let id = seed_run(&conn);

    // pending → ready
    let out = transition(
        &conn,
        &id,
        1,
        NodeRunStatus::Pending,
        NodeRunStatus::Ready,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(out, TransitionOutcome::Applied);
    let row = get_node_run(&conn, &id).unwrap().unwrap();
    assert_eq!(row.status, NodeRunStatus::Ready);
    assert_eq!(row.version, 2, "乐观锁 version 应 +1");

    // 旧版本再推进 → VersionConflict
    let out = transition(
        &conn,
        &id,
        1,
        NodeRunStatus::Ready,
        NodeRunStatus::Leased,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(out, TransitionOutcome::VersionConflict);
}

#[test]
fn illegal_transition_rejected() {
    let storage = mem();
    let conn = storage.conn();
    let id = seed_run(&conn);

    // pending → succeeded 非法（跳过全部中间态）
    let out = transition(
        &conn,
        &id,
        1,
        NodeRunStatus::Pending,
        NodeRunStatus::Succeeded,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(out, TransitionOutcome::IllegalTransition);
}

#[test]
fn idempotency_key_unique() {
    let storage = mem();
    let conn = storage.conn();
    seed_run(&conn);
    // 同 key 同 attempt 重复创建 → UNIQUE 冲突 → Sqlite 错误
    let dup = create_node_run(
        &conn,
        "g1",
        "a",
        NodeType::Executor,
        1,
        Some("p1"),
        Some("w1"),
    );
    assert!(
        dup.is_err(),
        "幂等键 (graph_run_id,node_key,attempt) 必须唯一"
    );
    // attempt=2 合法（新执行尝试）
    let (_, created) = create_node_run(
        &conn,
        "g1",
        "a",
        NodeType::Executor,
        2,
        Some("p1"),
        Some("w1"),
    )
    .unwrap();
    assert!(created);
}

#[test]
fn converge_rules() {
    let storage = mem();
    let conn = storage.conn();
    seed_run(&conn);
    let id = get_node_run_by_key(&conn, "g1", "a", 1)
        .unwrap()
        .unwrap()
        .id;

    // 活动节点 → 不收敛
    assert_eq!(converge_graph_run(&conn, "g1").unwrap(), None);

    // 全部 succeeded → completed
    transition(
        &conn,
        &id,
        1,
        NodeRunStatus::Pending,
        NodeRunStatus::Ready,
        None,
        None,
        None,
    )
    .unwrap();
    let row = get_node_run(&conn, &id).unwrap().unwrap();
    transition(
        &conn,
        &id,
        row.version,
        NodeRunStatus::Ready,
        NodeRunStatus::Leased,
        None,
        None,
        None,
    )
    .unwrap();
    let row = get_node_run(&conn, &id).unwrap().unwrap();
    transition(
        &conn,
        &id,
        row.version,
        NodeRunStatus::Leased,
        NodeRunStatus::Dispatched,
        None,
        None,
        None,
    )
    .unwrap();
    let row = get_node_run(&conn, &id).unwrap().unwrap();
    transition(
        &conn,
        &id,
        row.version,
        NodeRunStatus::Dispatched,
        NodeRunStatus::Running,
        None,
        None,
        Some(1.0),
    )
    .unwrap();
    let row = get_node_run(&conn, &id).unwrap().unwrap();
    transition(
        &conn,
        &id,
        row.version,
        NodeRunStatus::Running,
        NodeRunStatus::Verifying,
        None,
        None,
        None,
    )
    .unwrap();
    let row = get_node_run(&conn, &id).unwrap().unwrap();
    transition(
        &conn,
        &id,
        row.version,
        NodeRunStatus::Verifying,
        NodeRunStatus::Succeeded,
        None,
        None,
        None,
    )
    .unwrap();

    assert_eq!(
        converge_graph_run(&conn, "g1").unwrap(),
        Some(GraphRunStatus::Completed)
    );
    // 终态幂等：再调返回 None
    assert_eq!(converge_graph_run(&conn, "g1").unwrap(), None);
}

#[test]
fn converge_failed_when_node_failed() {
    let storage = mem();
    let conn = storage.conn();
    seed_run(&conn);
    let id = get_node_run_by_key(&conn, "g1", "a", 1)
        .unwrap()
        .unwrap()
        .id;

    transition(
        &conn,
        &id,
        1,
        NodeRunStatus::Pending,
        NodeRunStatus::Ready,
        None,
        None,
        None,
    )
    .unwrap();
    let row = get_node_run(&conn, &id).unwrap().unwrap();
    transition(
        &conn,
        &id,
        row.version,
        NodeRunStatus::Ready,
        NodeRunStatus::Leased,
        None,
        None,
        None,
    )
    .unwrap();
    let row = get_node_run(&conn, &id).unwrap().unwrap();
    transition(
        &conn,
        &id,
        row.version,
        NodeRunStatus::Leased,
        NodeRunStatus::Dispatched,
        None,
        None,
        None,
    )
    .unwrap();
    let row = get_node_run(&conn, &id).unwrap().unwrap();
    transition(
        &conn,
        &id,
        row.version,
        NodeRunStatus::Dispatched,
        NodeRunStatus::Running,
        None,
        None,
        None,
    )
    .unwrap();
    let row = get_node_run(&conn, &id).unwrap().unwrap();
    transition(
        &conn,
        &id,
        row.version,
        NodeRunStatus::Running,
        NodeRunStatus::Verifying,
        None,
        None,
        None,
    )
    .unwrap();
    let row = get_node_run(&conn, &id).unwrap().unwrap();
    transition(
        &conn,
        &id,
        row.version,
        NodeRunStatus::Verifying,
        NodeRunStatus::Failed,
        Some("test_failure"),
        Some("boom"),
        None,
    )
    .unwrap();

    assert_eq!(
        converge_graph_run(&conn, "g1").unwrap(),
        Some(GraphRunStatus::Failed)
    );
}
