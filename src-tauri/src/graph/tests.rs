//! Graph Compiler 测试（就近原则，独立 tests.rs：compiler.rs 实现保持行数红线内）。
//! 覆盖：合法 DAG、结构校验（重复 ID/依赖缺失/环/不可达）、契约校验（timeout/workspace/
//! participant/join/approval）、路径越界、并行冲突（write_paths 重叠/共享契约）、安全检查。

use crate::graph::compiler::{compile, CompileResult};
use crate::graph::spec::GraphSpec;

fn compile_yaml(yaml: &str) -> CompileResult {
    let spec = GraphSpec::parse(yaml).expect("yaml 应可解析");
    compile(&spec)
}

fn err_codes(result: &CompileResult) -> Vec<&'static str> {
    result.errors.iter().map(|e| e.code).collect()
}

fn warn_codes(result: &CompileResult) -> Vec<&'static str> {
    result.warnings.iter().map(|e| e.code).collect()
}

/// 合法的最小三节点串行 DAG：impl-backend → test-backend → verify。
fn three_node_dag() -> String {
    r#"
version: 1
goal: "后端实现与验证"
max_concurrency: 1
nodes:
  - id: impl-backend
    type: executor
    outputs: { schema: source-diff, artifacts: [backend-diff] }
    executor_requirements: { participant: backend-agent }
    workspace: w1
    read_paths: [src/backend]
    write_paths: [src/backend]
    forbidden_paths: [Cargo.lock]
    acceptance: [{ type: path, rule: changed_within_write_paths }]
    timeout_seconds: 300
    retry_policy: { max_attempts: 2, retryable: [execution_error, agent_timeout] }
  - id: test-backend
    type: deterministic
    dependencies: [impl-backend]
    inputs: [backend-diff]
    outputs: { schema: test-report, artifacts: [test-report] }
    read_paths: [src/backend]
    acceptance: [{ type: command, rule: "cargo test -p backend" }]
    timeout_seconds: 120
  - id: verify
    type: deterministic
    dependencies: [test-backend]
    outputs: { schema: verification-report }
    acceptance: [{ type: artifact, rule: checksum_matches }]
    timeout_seconds: 60
"#
    .to_string()
}

#[test]
fn valid_dag_compiles() {
    let r = compile_yaml(&three_node_dag());
    assert!(r.valid, "合法 DAG 应编译通过：{:?}", r.errors);
    let c = r.compiled.unwrap();
    assert_eq!(
        c.execution_order,
        vec!["impl-backend", "test-backend", "verify"]
    );
    assert_eq!(c.edges.len(), 2);
    assert!(c.required_approvals.is_empty());
    // 三节点串行 → 每层一个节点
    assert_eq!(c.parallel_groups.len(), 3);
}

#[test]
fn duplicate_node_id_rejected() {
    let yaml = three_node_dag().replace("  - id: test-backend", "  - id: impl-backend");
    let r = compile_yaml(&yaml);
    assert!(!r.valid);
    assert!(err_codes(&r).contains(&"duplicate_node_id"));
}

#[test]
fn missing_dependency_rejected() {
    let yaml = three_node_dag().replace("dependencies: [impl-backend]", "dependencies: [ghost]");
    let r = compile_yaml(&yaml);
    assert!(!r.valid);
    assert!(err_codes(&r).contains(&"edge_target_missing"));
}

#[test]
fn cycle_detected() {
    // a → b → a
    let yaml = r#"
version: 1
goal: "环"
nodes:
  - id: a
    type: executor
    dependencies: [b]
    outputs: { schema: s }
    executor_requirements: { participant: p1 }
    workspace: w1
    write_paths: [x]
    acceptance: [{ type: path }]
    timeout_seconds: 10
  - id: b
    type: executor
    dependencies: [a]
    outputs: { schema: s }
    executor_requirements: { participant: p1 }
    workspace: w1
    write_paths: [x]
    acceptance: [{ type: path }]
    timeout_seconds: 10
"#;
    let r = compile_yaml(yaml);
    assert!(!r.valid);
    assert!(err_codes(&r).contains(&"cycle_detected"));
}

#[test]
fn writer_without_workspace_rejected() {
    let yaml = three_node_dag().replace("    workspace: w1\n", "");
    let r = compile_yaml(&yaml);
    assert!(!r.valid);
    assert!(err_codes(&r).contains(&"workspace_required"));
}

#[test]
fn missing_timeout_rejected() {
    let yaml = three_node_dag().replace("    timeout_seconds: 300\n", "    timeout_seconds: 0\n");
    let r = compile_yaml(&yaml);
    assert!(!r.valid);
    assert!(err_codes(&r).contains(&"timeout_required"));
}

#[test]
fn executor_without_participant_allowed_with_auto_warning() {
    // participant 缺省/auto → 编译通过（daemon 提交时自动分配随机执行者），warning 提示
    let yaml = three_node_dag().replace(
        "    executor_requirements: { participant: backend-agent }\n",
        "",
    );
    let r = compile_yaml(&yaml);
    assert!(r.valid, "缺 participant 不应编译失败（auto 分配）");
    assert!(warn_codes(&r).contains(&"auto_participant"));

    let yaml2 = three_node_dag().replace(
        "    executor_requirements: { participant: backend-agent }\n",
        "    executor_requirements: { participant: auto }\n",
    );
    let r2 = compile_yaml(&yaml2);
    assert!(r2.valid);
    assert!(warn_codes(&r2).contains(&"auto_participant"));
}

#[test]
fn join_requires_policy_and_upstream() {
    let yaml = r#"
version: 1
goal: "join"
nodes:
  - id: j
    type: join
    acceptance: []
    timeout_seconds: 10
"#;
    let r = compile_yaml(yaml);
    assert!(!r.valid);
    let codes = err_codes(&r);
    assert!(codes.contains(&"join_policy_required"));
    assert!(codes.contains(&"join_needs_upstream"));
}

#[test]
fn valid_join_compiles() {
    let yaml = r#"
version: 1
goal: "join ok"
nodes:
  - id: a
    type: deterministic
    outputs: { schema: s }
    acceptance: [{ type: path }]
    timeout_seconds: 10
  - id: b
    type: deterministic
    outputs: { schema: s }
    acceptance: [{ type: path }]
    timeout_seconds: 10
  - id: j
    type: join
    dependencies: [a, b]
    join_policy: all_succeeded
    timeout_seconds: 10
"#;
    let r = compile_yaml(yaml);
    assert!(r.valid, "合法 join 应编译通过：{:?}", r.errors);
    // 并行候选：a、b 同层
    let c = r.compiled.unwrap();
    assert!(c
        .parallel_groups
        .iter()
        .any(|g| g.contains(&"a".to_string()) && g.contains(&"b".to_string())));
}

#[test]
fn approval_requires_message() {
    let yaml = r#"
version: 1
goal: "approval"
nodes:
  - id: ap
    type: approval
    timeout_seconds: 10
"#;
    let r = compile_yaml(yaml);
    assert!(!r.valid);
    assert!(err_codes(&r).contains(&"approval_message_required"));
}

#[test]
fn valid_approval_is_required_approval() {
    let yaml = r#"
version: 1
goal: "approval ok"
nodes:
  - id: ap
    type: approval
    approval: { message: "允许发布？", options: [yes, no] }
    timeout_seconds: 3600
"#;
    let r = compile_yaml(yaml);
    assert!(r.valid, "合法 approval 应编译通过：{:?}", r.errors);
    assert_eq!(r.compiled.unwrap().required_approvals, vec!["ap"]);
}

#[test]
fn overlapping_writers_conflict() {
    let yaml = r#"
version: 1
goal: "conflict"
max_concurrency: 4
nodes:
  - id: a
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p1 }
    workspace: w1
    write_paths: [src/backend]
    acceptance: [{ type: path }]
    timeout_seconds: 10
  - id: b
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p2 }
    workspace: w2
    write_paths: [src/backend/api.rs]
    acceptance: [{ type: path }]
    timeout_seconds: 10
"#;
    let r = compile_yaml(yaml);
    assert!(r.valid);
    assert!(
        r.compiled
            .unwrap()
            .resource_conflicts
            .iter()
            .any(|c| c.contains("'a'") && c.contains("'b'")),
        "write_paths 重叠应产生资源冲突记录"
    );
}

#[test]
fn shared_contract_conflict() {
    let yaml = r#"
version: 1
goal: "lockfile conflict"
max_concurrency: 4
nodes:
  - id: a
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p1 }
    workspace: w1
    write_paths: [Cargo.lock]
    acceptance: [{ type: path }]
    timeout_seconds: 10
  - id: b
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p2 }
    workspace: w2
    write_paths: [package-lock.json]
    acceptance: [{ type: path }]
    timeout_seconds: 10
"#;
    let r = compile_yaml(yaml);
    assert!(r.valid);
    assert!(warn_codes(&r).contains(&"shared_contract_serialized"));
}

#[test]
fn path_traversal_rejected() {
    let yaml = three_node_dag().replace(
        "    forbidden_paths: [Cargo.lock]\n",
        "    forbidden_paths: [../secret]\n",
    );
    let r = compile_yaml(&yaml);
    assert!(!r.valid);
    assert!(err_codes(&r).contains(&"path_traversal"));
}

#[test]
fn unreachable_node_warns() {
    let yaml = three_node_dag()
        + "\n  - id: orphan\n    type: deterministic\n    outputs: { schema: s }\n    acceptance: [{ type: path }]\n    timeout_seconds: 10\n";
    let r = compile_yaml(&yaml);
    assert!(r.valid);
    assert!(warn_codes(&r).contains(&"unreachable_node"));
}

#[test]
fn writer_without_forbidden_paths_warns() {
    let yaml = three_node_dag().replace("    forbidden_paths: [Cargo.lock]\n", "");
    let r = compile_yaml(&yaml);
    assert!(r.valid);
    assert!(warn_codes(&r).contains(&"forbidden_paths_recommended"));
}

#[test]
fn unbounded_retries_warns() {
    let yaml = three_node_dag().replace(
        "retry_policy: { max_attempts: 2, retryable: [execution_error, agent_timeout] }",
        "retry_policy: { max_attempts: 99 }",
    );
    let r = compile_yaml(&yaml);
    assert!(r.valid);
    assert!(warn_codes(&r).contains(&"unbounded_retries"));
}
