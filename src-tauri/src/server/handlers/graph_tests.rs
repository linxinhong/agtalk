#[cfg(test)]
mod tests {
    use crate::graph::state::{get_node_run_by_key, NodeRunStatus};
    use crate::identity::auth::AuthenticatedSession;
    use crate::proto::ServerMsg;
    use crate::server::handlers::graph::{
        apply_heartbeat, apply_result, control_run, delete_run, patch_run, show_run,
        submit_and_start, NodeBody,
    };
    use crate::server::state::AppState;
    use crate::storage::Storage;
    use rusqlite::params;

    /// 测试用 AppState（不启用 popup/feishu；预置 sender mailbox 供派发消息的 from 外键）。
    fn test_state() -> AppState {
        let storage = Storage::open_in_memory().unwrap();
        {
            let conn = storage.conn();
            conn.execute(
                "INSERT INTO mailboxes (address, name) VALUES ('sender-addr', 'sender')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO event_sequences (address) VALUES ('sender-addr')",
                [],
            )
            .unwrap();
        }
        AppState::new(
            storage,
            crate::config::AgConfig::default(),
            std::path::PathBuf::from("/tmp/test/.agtalk"),
        )
    }

    fn fake_session() -> AuthenticatedSession {
        AuthenticatedSession {
            address: "sender-addr".into(),
            name: "sender".into(),
            workspace: String::new(),
            workspace_root: std::path::PathBuf::from("/tmp/test/.agtalk"),
            pid: None,
        }
    }

    #[test]
    fn submit_valid_spec_creates_run_and_dispatches_first_node() {
        let state = test_state();
        // participant 在线：注册 backend-agent mailbox
        let yaml = r#"
version: 1
goal: "三节点串行验证"
nodes:
  - id: impl
    type: executor
    outputs: { schema: source-diff }
    executor_requirements: { participant: backend-agent }
    workspace: w1
    write_paths: [src/backend]
    acceptance: [{ type: path }]
    timeout_seconds: 300
  - id: verify
    type: deterministic
    dependencies: [impl]
    outputs: { schema: test-report }
    acceptance: [{ type: command }]
    timeout_seconds: 60
"#;
        // 注册 participant
        {
            let conn = state.storage.conn();
            conn.execute(
                "INSERT INTO mailboxes (address, name) VALUES ('p-addr', 'backend-agent')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO event_sequences (address) VALUES ('p-addr')",
                [],
            )
            .unwrap();
        }

        let msg = submit_and_start(&state, &fake_session(), yaml).unwrap();
        match msg {
            ServerMsg::GraphRunCreated {
                run_id,
                status,
                errors,
                ..
            } => {
                assert!(!run_id.is_empty());
                assert_eq!(status, "ready");
                assert!(errors.is_empty(), "不应有编译错误");
                // 第一个节点应已派发（状态 dispatched + 派发消息）
                let conn = state.storage.conn();
                let impl_run = get_node_run_by_key(&conn, &run_id, "impl", 1)
                    .unwrap()
                    .unwrap();
                assert_eq!(impl_run.status, NodeRunStatus::Dispatched);
                // 派发消息已到 backend-agent
                let cnt: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM messages WHERE to_address='p-addr'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(cnt, 1, "应有一条派发消息");
                let assignment: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM graph_node_assignments WHERE node_run_id=?1",
                        params![impl_run.id],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(assignment, 1);
            }
            other => panic!("预期 GraphRunCreated，实际 {other:?}"),
        }
    }

    #[test]
    fn submit_invalid_spec_reports_errors() {
        let state = test_state();
        let yaml = r#"
version: 1
goal: ""
nodes:
  - id: a
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p }
    workspace: w
    write_paths: [x]
    acceptance: [{ type: path }]
    timeout_seconds: 10
"#;
        match submit_and_start(&state, &fake_session(), yaml).unwrap() {
            ServerMsg::GraphRunCreated { status, errors, .. } => {
                assert_eq!(status, "invalid");
                assert!(!errors.is_empty());
                assert!(errors.iter().any(|e| e.code == "goal_required"));
            }
            other => panic!("预期 GraphRunCreated(invalid)，实际 {other:?}"),
        }
    }

    #[test]
    fn heartbeat_and_result_full_cycle() {
        let state = test_state();
        // 建 run + 单节点（participant 在线）
        let yaml = r#"
version: 1
goal: "单节点"
nodes:
  - id: only
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: agent-x }
    workspace: w
    write_paths: [x]
    acceptance: [{ type: path }]
    timeout_seconds: 300
"#;
        {
            let conn = state.storage.conn();
            conn.execute(
                "INSERT INTO mailboxes (address, name) VALUES ('x-addr', 'agent-x')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO event_sequences (address) VALUES ('x-addr')",
                [],
            )
            .unwrap();
        }

        let run_id = match submit_and_start(&state, &fake_session(), yaml).unwrap() {
            ServerMsg::GraphRunCreated { run_id, .. } => run_id,
            _ => panic!(),
        };

        // M2 验证需要真实存在的 artifact 文件（checksum 校验）
        let dir = tempfile::tempdir().unwrap();
        let artifact_path = dir.path().join("report.json");
        std::fs::write(&artifact_path, "{}").unwrap();

        // heartbeat：dispatched → running
        let hb = apply_heartbeat(
            &state,
            &NodeBody {
                run_id: run_id.clone(),
                node_key: "only".into(),
                attempt: 1,
                result: String::new(),
                changed_files: vec![],
                output_artifacts: vec![],
                verification_claims: vec![],
                blockers: vec![],
            },
        )
        .unwrap();
        assert!(matches!(hb, ServerMsg::GraphNodeReportOk { status, .. } if status == "running"));

        // result → succeeded，图收敛 completed
        let res = apply_result(
            &state,
            &fake_session(),
            &NodeBody {
                run_id: run_id.clone(),
                node_key: "only".into(),
                attempt: 1,
                result: "done".into(),
                changed_files: vec!["x/a.rs".into()],
                output_artifacts: vec![crate::graph::dto::GraphArtifactRef {
                    artifact_type: "source-diff".into(),
                    schema_version: "v1".into(),
                    uri: format!("file://{}", artifact_path.display()),
                    checksum: String::new(),
                }],
                verification_claims: vec![crate::graph::dto::GraphClaim {
                    command: "cargo test".into(),
                    exit_code: 0,
                    stdout_ref: String::new(),
                    summary: "ok".into(),
                }],
                blockers: vec![],
            },
        )
        .unwrap();
        assert!(
            matches!(res, ServerMsg::GraphNodeReportOk { status, .. } if status == "succeeded")
        );

        // 图应已 completed
        let status: String = {
            let conn = state.storage.conn();
            conn.query_row(
                "SELECT status FROM graph_runs WHERE id=?1",
                params![run_id],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(status, "completed");
        // claims 已落库
        let v: i64 = {
            let conn = state.storage.conn();
            conn.query_row("SELECT COUNT(*) FROM verifications", [], |r| r.get(0))
                .unwrap()
        };
        assert!(v >= 1, "claims 已落库");
    }

    #[test]
    fn result_with_blocker_blocks_node() {
        let state = test_state();
        let yaml = r#"
version: 1
goal: "blocker"
nodes:
  - id: only
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: agent-x }
    workspace: w
    write_paths: [x]
    acceptance: [{ type: path }]
    timeout_seconds: 300
"#;
        {
            let conn = state.storage.conn();
            conn.execute(
                "INSERT INTO mailboxes (address, name) VALUES ('x-addr', 'agent-x')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO event_sequences (address) VALUES ('x-addr')",
                [],
            )
            .unwrap();
        }
        let run_id = match submit_and_start(&state, &fake_session(), yaml).unwrap() {
            ServerMsg::GraphRunCreated { run_id, .. } => run_id,
            _ => panic!(),
        };
        apply_heartbeat(
            &state,
            &NodeBody {
                run_id: run_id.clone(),
                node_key: "only".into(),
                attempt: 1,
                result: String::new(),
                changed_files: vec![],
                output_artifacts: vec![],
                verification_claims: vec![],
                blockers: vec![],
            },
        )
        .unwrap();
        let res = apply_result(
            &state,
            &fake_session(),
            &NodeBody {
                run_id: run_id.clone(),
                node_key: "only".into(),
                attempt: 1,
                result: String::new(),
                changed_files: vec![],
                output_artifacts: vec![],
                verification_claims: vec![],
                blockers: vec!["需要人类决策".into()],
            },
        )
        .unwrap();
        assert!(matches!(res, ServerMsg::GraphNodeReportOk { status, .. } if status == "blocked"));
    }
    #[test]
    fn failed_node_retries_with_new_attempt() {
        // M2 验收：验证失败且 retryable → 新建 attempt（不覆盖失败记录）
        let state = test_state();
        let yaml = r#"
version: 1
goal: "retry"
nodes:
  - id: only
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: agent-x }
    workspace: w
    write_paths: [x]
    acceptance: [{ type: path }]
    timeout_seconds: 300
    retry_policy: { max_attempts: 2, retryable: [contract_violation] }
"#;
        {
            let conn = state.storage.conn();
            conn.execute(
                "INSERT INTO mailboxes (address, name) VALUES ('x-addr', 'agent-x')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO event_sequences (address) VALUES ('x-addr')",
                [],
            )
            .unwrap();
        }
        let run_id = match submit_and_start(&state, &fake_session(), yaml).unwrap() {
            ServerMsg::GraphRunCreated { run_id, .. } => run_id,
            _ => panic!(),
        };
        apply_heartbeat(
            &state,
            &NodeBody {
                run_id: run_id.clone(),
                node_key: "only".into(),
                attempt: 1,
                result: String::new(),
                changed_files: vec![],
                output_artifacts: vec![],
                verification_claims: vec![],
                blockers: vec![],
            },
        )
        .unwrap();

        // result 为空 → contract_violation → 重试 attempt 2
        let res = apply_result(
            &state,
            &fake_session(),
            &NodeBody {
                run_id: run_id.clone(),
                node_key: "only".into(),
                attempt: 1,
                result: String::new(), // 空 result = contract_violation
                changed_files: vec![],
                output_artifacts: vec![],
                verification_claims: vec![],
                blockers: vec![],
            },
        )
        .unwrap();
        assert!(
            matches!(res, ServerMsg::GraphNodeReportOk { status, .. } if status == "retrying"),
            "contract_violation 且 retryable 应触发重试"
        );

        let conn = state.storage.conn();
        let a1 = get_node_run_by_key(&conn, &run_id, "only", 1)
            .unwrap()
            .unwrap();
        assert_eq!(a1.status, NodeRunStatus::Failed, "attempt 1 保留失败记录");
        assert_eq!(a1.failure_type.as_deref(), Some("contract_violation"));
        let a2 = get_node_run_by_key(&conn, &run_id, "only", 2)
            .unwrap()
            .unwrap();
        assert_eq!(
            a2.status,
            NodeRunStatus::Dispatched,
            "attempt 2 已建并自动重新派发（重试后 tick）"
        );
        // 验证结果落库
        let v: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM verifications WHERE node_run_id=?1",
                params![a1.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(v >= 1, "验证记录应落库");
    }

    #[test]
    fn control_run_pushes_events_to_hub() {
        // M3：GraphEvent 先落库后推送（hub flush 机制）
        let state = test_state();
        {
            let conn = state.storage.conn();
            conn.execute(
                "INSERT INTO graph_runs (id, goal, spec_snapshot, compiled_graph, status) \
             VALUES ('g-cancel', 'g', '{}', '{}', 'running')",
                [],
            )
            .unwrap();
        }
        let mut rx = state.graph_events.subscribe("g-cancel");
        let res =
            crate::server::handlers::graph::control_run(&state, "g-cancel", "cancel").unwrap();
        assert!(
            matches!(res, ServerMsg::GraphRunControlOk { status, .. } if status == "cancelled")
        );
        // 推送应包含 graph_cancelled（先落库，flush 到 hub）
        let evt = rx.try_recv().unwrap();
        assert_eq!(evt.event_type, "graph_cancelled");
        assert_eq!(evt.node_key, None);
    }

    #[test]
    fn repair_flow_failure_triggers_repair_then_retry_original() {
        // P1-3：a 失败（有修复者 b）→ 走 Repair（不自动重试）→ b 派发 → b 成功 → a attempt2 重试
        let state = test_state();
        {
            let conn = state.storage.conn();
            conn.execute(
                "INSERT INTO mailboxes (address, name) VALUES ('x-addr', 'agent-x')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO event_sequences (address) VALUES ('x-addr')",
                [],
            )
            .unwrap();
        }
        let yaml = r#"
version: 1
goal: "repair flow"
nodes:
  - id: a
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: agent-x }
    workspace: w1
    write_paths: [src/a]
    acceptance: [{ type: path }]
    timeout_seconds: 300
  - id: b
    type: executor
    on_failure: [a]
    outputs: { schema: s }
    executor_requirements: { participant: agent-x }
    workspace: w2
    write_paths: [src/a]
    acceptance: [{ type: path }]
    timeout_seconds: 300
"#;
        let run_id = match submit_and_start(&state, &fake_session(), yaml).unwrap() {
            ServerMsg::GraphRunCreated { run_id, .. } => run_id,
            _ => panic!(),
        };

        // a 执行但结果为空 → contract_violation → 进入修复（b 被派发）
        apply_heartbeat(
            &state,
            &NodeBody {
                run_id: run_id.clone(),
                node_key: "a".into(),
                attempt: 1,
                result: String::new(),
                changed_files: vec![],
                output_artifacts: vec![],
                verification_claims: vec![],
                blockers: vec![],
            },
        )
        .unwrap();
        let res = apply_result(
            &state,
            &fake_session(),
            &NodeBody {
                run_id: run_id.clone(),
                node_key: "a".into(),
                attempt: 1,
                result: String::new(), // 空 result = contract_violation
                changed_files: vec![],
                output_artifacts: vec![],
                verification_claims: vec![],
                blockers: vec![],
            },
        )
        .unwrap();
        assert!(
            matches!(res, ServerMsg::GraphNodeReportOk { status, .. } if status == "failed_repairing"),
            "a 失败应进入修复流程"
        );
        // b 被派发（on_failure 触发）
        let b1 = {
            let conn = state.storage.conn();
            crate::graph::state::get_node_run_by_key(&conn, &run_id, "b", 1)
                .unwrap()
                .unwrap()
        };
        assert_eq!(
            b1.status,
            crate::graph::state::NodeRunStatus::Dispatched,
            "修复节点 b 应被派发"
        );

        // b 修复成功
        apply_heartbeat(
            &state,
            &NodeBody {
                run_id: run_id.clone(),
                node_key: "b".into(),
                attempt: 1,
                result: String::new(),
                changed_files: vec![],
                output_artifacts: vec![],
                verification_claims: vec![],
                blockers: vec![],
            },
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("fix.txt");
        std::fs::write(&f, "fix").unwrap();
        apply_result(
            &state,
            &fake_session(),
            &NodeBody {
                run_id: run_id.clone(),
                node_key: "b".into(),
                attempt: 1,
                result: "修复完成".into(),
                changed_files: vec!["src/a/fix.rs".into()],
                output_artifacts: vec![crate::graph::dto::GraphArtifactRef {
                    artifact_type: "source-diff".into(),
                    schema_version: "v1".into(),
                    uri: format!("file://{}", f.display()),
                    checksum: String::new(),
                }],
                verification_claims: vec![],
                blockers: vec![],
            },
        )
        .unwrap();

        // b succeeded + a attempt2 被触发重试并派发
        let conn = state.storage.conn();
        let b = crate::graph::state::get_node_run_by_key(&conn, &run_id, "b", 1)
            .unwrap()
            .unwrap();
        assert_eq!(b.status, crate::graph::state::NodeRunStatus::Succeeded);
        let a2 = crate::graph::state::get_node_run_by_key(&conn, &run_id, "a", 2)
            .unwrap()
            .unwrap();
        assert_eq!(
            a2.status,
            crate::graph::state::NodeRunStatus::Dispatched,
            "修复后原节点 a 应重试（attempt 2）"
        );
        // a attempt1 保留失败记录
        let a1 = crate::graph::state::get_node_run_by_key(&conn, &run_id, "a", 1)
            .unwrap()
            .unwrap();
        assert_eq!(a1.status, crate::graph::state::NodeRunStatus::Failed);
    }

    #[test]
    fn patch_updates_paused_graph_definition() {
        // P2：paused 图可 patch（定义替换 + 重新编译 + 事件）
        let state = test_state();
        {
            let conn = state.storage.conn();
            conn.execute(
                "INSERT INTO graph_runs (id, goal, spec_snapshot, compiled_graph, status) \
             VALUES ('g-patch', 'old', '{}', '{}', 'paused')",
                [],
            )
            .unwrap();
        }
        let yaml = r#"
version: 1
goal: "patched 新目标"
nodes:
  - id: only
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: agent-x }
    workspace: w
    write_paths: [x]
    acceptance: [{ type: path }]
    timeout_seconds: 300
"#;
        match patch_run(&state, "g-patch", yaml).unwrap() {
            ServerMsg::GraphRunControlOk { action, status, .. } => {
                assert_eq!(action, "patch");
                assert_eq!(status, "patched");
            }
            other => panic!("预期 GraphRunControlOk: {other:?}"),
        }
        let conn = state.storage.conn();
        let (goal, cg): (String, String) = conn
            .query_row(
                "SELECT goal, compiled_graph FROM graph_runs WHERE id='g-patch'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(goal, "patched 新目标", "goal 应更新");
        assert!(cg.contains("only"), "compiled_graph 应包含新节点");
        let ev: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM graph_events WHERE graph_run_id='g-patch' AND event_type='graph_patched'",
            [],
            |r| r.get(0),
        )
        .unwrap();
        assert_eq!(ev, 1, "应有 graph_patched 事件");
    }

    #[test]
    fn patch_rejects_running_and_terminal() {
        let state = test_state();
        let yaml = r#"
version: 1
goal: "g"
nodes:
  - id: n
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p }
    workspace: w
    write_paths: [x]
    acceptance: [{ type: path }]
    timeout_seconds: 300
"#;
        for (id, status) in [("g-run", "running"), ("g-done", "completed")] {
            {
                let conn = state.storage.conn();
                conn.execute(
                    "INSERT INTO graph_runs (id, goal, spec_snapshot, compiled_graph, status) \
                 VALUES (?1, 'g', '{}', '{}', ?2)",
                    rusqlite::params![id, status],
                )
                .unwrap();
            }
            match patch_run(&state, id, yaml) {
                Err(ServerMsg::Error { code, .. }) => {
                    assert!(
                        code == "graph_patch_running" || code == "graph_patch_terminal",
                        "应拒绝 {status} 图: {code}"
                    );
                }
                other => panic!("预期拒绝 {status} 图: {other:?}"),
            }
        }
    }

    #[test]
    fn show_run_reports_participant_online_state() {
        // 认领状态着色数据：participant 有活跃 mailbox → participant_online=true；
        // 离线/不存在 → false；无 participant（控制节点）→ false
        let state = test_state();
        {
            let conn = state.storage.conn();
            conn.execute(
                "INSERT INTO mailboxes (address, name) VALUES ('x-addr', 'agent-x')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO event_sequences (address) VALUES ('x-addr')",
                [],
            )
            .unwrap();
        }
        let yaml = r#"
version: 1
goal: "claim status"
nodes:
  - id: online
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: agent-x }
    workspace: w1
    write_paths: [src/online]
    acceptance: [{ type: path }]
    timeout_seconds: 300
  - id: offline
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: ghost-agent }
    workspace: w2
    write_paths: [src/offline]
    acceptance: [{ type: path }]
    timeout_seconds: 300
"#;
        let run_id = match submit_and_start(&state, &fake_session(), yaml).unwrap() {
            ServerMsg::GraphRunCreated { run_id, .. } => run_id,
            _ => panic!("submit 失败"),
        };
        let msg = show_run(&state, &run_id).unwrap();
        let ServerMsg::GraphRunDetail { nodes, .. } = msg else {
            panic!("期望 GraphRunDetail");
        };
        let by_key = |k: &str| nodes.iter().find(|n| n.node_key == k).unwrap();
        assert!(
            by_key("online").participant_online,
            "在线 participant 应报 online"
        );
        assert!(
            !by_key("offline").participant_online,
            "离线 participant 应报 offline"
        );
    }

    #[test]
    fn delete_run_removes_cascading_data() {
        let state = test_state();
        {
            let conn = state.storage.conn();
            conn.execute(
                "INSERT INTO mailboxes (address, name) VALUES ('x-addr', 'agent-x')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO event_sequences (address) VALUES ('x-addr')",
                [],
            )
            .unwrap();
        }
        let yaml = r#"
version: 1
goal: "delete test"
nodes:
  - id: only
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: agent-x }
    workspace: w1
    write_paths: [src/x]
    acceptance: [{ type: path }]
    timeout_seconds: 300
"#;
        let run_id = match submit_and_start(&state, &fake_session(), yaml).unwrap() {
            ServerMsg::GraphRunCreated { run_id, .. } => run_id,
            _ => panic!(),
        };
        // cancel 后（terminal）才可删
        let _ = control_run(&state, &run_id, "cancel").unwrap();
        let msg = delete_run(&state, &run_id).unwrap();
        assert!(matches!(msg, ServerMsg::GraphRunDeleted { .. }));
        // 级联：node_runs / graph_events 已清空
        let conn = state.storage.conn();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM node_runs WHERE graph_run_id=?1",
                params![run_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "node_runs 应级联删除");
        let e: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM graph_events WHERE graph_run_id=?1",
                params![run_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(e, 0, "graph_events 应级联删除");
        let g: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM graph_runs WHERE id=?1",
                params![run_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(g, 0, "graph_runs 应删除");
    }

    #[test]
    fn delete_run_rejects_non_terminal() {
        let state = test_state();
        {
            let conn = state.storage.conn();
            conn.execute(
                "INSERT INTO mailboxes (address, name) VALUES ('x-addr', 'agent-x')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO event_sequences (address) VALUES ('x-addr')",
                [],
            )
            .unwrap();
        }
        let yaml = r#"
version: 1
goal: "delete running test"
nodes:
  - id: only
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: agent-x }
    workspace: w1
    write_paths: [src/x]
    acceptance: [{ type: path }]
    timeout_seconds: 300
"#;
        let run_id = match submit_and_start(&state, &fake_session(), yaml).unwrap() {
            ServerMsg::GraphRunCreated { run_id, .. } => run_id,
            _ => panic!(),
        };
        // 未 cancel（ready/非 terminal）→ 拒绝
        let err = delete_run(&state, &run_id).unwrap_err();
        let ServerMsg::Error { code, .. } = err else {
            panic!("应返回错误")
        };
        assert_eq!(code, "graph_delete_non_terminal");
    }
}
