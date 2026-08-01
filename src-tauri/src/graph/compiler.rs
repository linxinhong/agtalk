//! Graph Compiler（docs/design_graph.md §三、design_graph.md §5.1 compiler.rs）。
//!
//! 任何任务图必须先经过编译才能运行：结构校验 / 契约校验 / 并行冲突校验 / 安全检查。
//! 输出结构化结果（valid / errors / warnings / compiled），无错误时产出 CompiledGraph。
//! 路径工具在 paths.rs，测试在 tests.rs（就近原则，避免单文件超行数红线）。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::paths::{paths_overlap, touches_shared_contract};
use super::spec::{GraphSpec, NodeSpec, NodeType};

/// 边触发条件（docs/design_graph.md §四 Edge）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trigger {
    OnSuccess,
    OnFailure,
    OnBlocked,
    Always,
}

impl Trigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Trigger::OnSuccess => "on_success",
            Trigger::OnFailure => "on_failure",
            Trigger::OnBlocked => "on_blocked",
            Trigger::Always => "always",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub trigger: Trigger,
}

/// 编译诊断项（error 或 warning）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompileIssue {
    pub code: &'static str,
    pub message: String,
    pub node: Option<String>,
}

/// 编译通过后的不可变图（运行时快照存 JSON）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompiledGraph {
    pub goal: String,
    pub repository: Option<String>,
    pub base_revision: Option<String>,
    pub integration_target: Option<String>,
    pub max_concurrency: u32,
    pub nodes: Vec<NodeSpec>,
    pub edges: Vec<Edge>,
    /// 拓扑执行顺序（基于 on_success 依赖）。
    pub execution_order: Vec<String>,
    /// 拓扑层分组：同组节点无依赖关系（并行候选，未扣除写冲突）。
    pub parallel_groups: Vec<Vec<String>>,
    /// 需要人工/外部审批的节点（type=approval）。
    pub required_approvals: Vec<String>,
    /// 检测到的资源冲突描述（M1 并行调度时参考；M0 串行不阻塞）。
    pub resource_conflicts: Vec<String>,
    /// 结构化冲突对（节点 key 对；并行调度时同轮内不得同时派发）。
    pub conflict_pairs: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompileResult {
    pub valid: bool,
    pub errors: Vec<CompileIssue>,
    pub warnings: Vec<CompileIssue>,
    pub compiled: Option<CompiledGraph>,
}

/// 编译入口。
pub fn compile(spec: &GraphSpec) -> CompileResult {
    let mut errors: Vec<CompileIssue> = Vec::new();
    let mut warnings: Vec<CompileIssue> = Vec::new();

    // ---- 1. 基本结构 ----
    if spec.version != 1 {
        errors.push(issue(
            "unsupported_spec_version",
            "spec version 必须为 1",
            None,
        ));
    }
    if spec.goal.trim().is_empty() {
        errors.push(issue("goal_required", "goal 不能为空", None));
    }
    if spec.nodes.is_empty() {
        errors.push(issue("nodes_required", "图至少需要一个节点", None));
    }
    if spec.max_concurrency == 0 {
        errors.push(issue(
            "invalid_max_concurrency",
            "max_concurrency 必须 ≥ 1",
            None,
        ));
    }

    // 节点索引 + ID 唯一性
    let mut by_id: BTreeMap<&str, &NodeSpec> = BTreeMap::new();
    for n in &spec.nodes {
        if by_id.insert(n.id.as_str(), n).is_some() {
            errors.push(issue(
                "duplicate_node_id",
                format!("节点 ID '{}' 重复", n.id),
                Some(n.id.clone()),
            ));
        }
    }
    if !errors.is_empty() {
        return finish(spec, errors, warnings);
    }

    // ---- 2. 边收集与目标存在性 ----
    let mut edges: Vec<Edge> = Vec::new();
    for n in &spec.nodes {
        for (targets, trigger) in [
            (&n.dependencies, Trigger::OnSuccess),
            (&n.on_failure, Trigger::OnFailure),
            (&n.on_blocked, Trigger::OnBlocked),
            (&n.always, Trigger::Always),
        ] {
            for to in targets {
                if !by_id.contains_key(to.as_str()) {
                    errors.push(issue(
                        "edge_target_missing",
                        format!(
                            "节点 '{}' 的 {} 边指向不存在的节点 '{}'",
                            n.id,
                            trigger.as_str(),
                            to
                        ),
                        Some(n.id.clone()),
                    ));
                } else if to == &n.id {
                    errors.push(issue(
                        "self_dependency",
                        format!("节点 '{}' 不能依赖自身", n.id),
                        Some(n.id.clone()),
                    ));
                } else {
                    edges.push(Edge {
                        from: n.id.clone(),
                        to: to.clone(),
                        trigger,
                    });
                }
            }
        }
    }

    // ---- 3. 环检测（所有边参与：只支持 DAG） ----
    let mut adj: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in &edges {
        adj.entry(e.from.as_str()).or_default().push(e.to.as_str());
    }
    if let Some(cycle) = find_cycle(&spec.nodes, &adj) {
        errors.push(issue(
            "cycle_detected",
            format!("图中存在环：{}", cycle.join(" -> ")),
            None,
        ));
    }

    // ---- 4. 契约校验（逐节点） ----
    for n in &spec.nodes {
        contract_check(n, &mut errors, &mut warnings);
    }

    // ---- 5. 输入 Artifact 引用（宽松：warning，可能由外部提供） ----
    for n in &spec.nodes {
        if n.inputs.is_empty() {
            continue;
        }
        let produced_by_deps: BTreeSet<&str> = spec
            .nodes
            .iter()
            .filter(|u| n.dependencies.iter().any(|d| d == &u.id))
            .flat_map(|u| u.outputs.artifacts.iter().map(String::as_str))
            .collect();
        for input in &n.inputs {
            if !produced_by_deps.contains(input.as_str()) {
                warnings.push(issue(
                    "input_artifact_unreferenced",
                    format!(
                        "节点 '{}' 的输入 artifact '{}' 未被直接上游产出（可能由外部提供）",
                        n.id, input
                    ),
                    Some(n.id.clone()),
                ));
            }
        }
    }

    // ---- 6. 路径合法性（read/write/forbidden 均拒绝 `..` 段） ----
    for n in &spec.nodes {
        for (kind, paths) in [
            ("read_paths", &n.read_paths),
            ("write_paths", &n.write_paths),
            ("forbidden_paths", &n.forbidden_paths),
        ] {
            for p in paths {
                if p.split('/').any(|seg| seg == "..") {
                    errors.push(issue(
                        "path_traversal",
                        format!("节点 '{}' 的 {kind} 含非法 '..' 段：{p}", n.id),
                        Some(n.id.clone()),
                    ));
                }
            }
        }
    }

    if !errors.is_empty() {
        return finish(spec, errors, warnings);
    }

    // ---- 7. 拓扑排序（Kahn，基于 on_success 依赖） ----
    let order = topo_order(&spec.nodes);
    let parallel_groups = topo_levels(&spec.nodes);
    let (conflict_pairs, conflicts, mut extra_warnings) = parallel_conflicts(spec);
    warnings.append(&mut extra_warnings);
    let required_approvals: Vec<String> = spec
        .nodes
        .iter()
        .filter(|n| n.node_type == NodeType::Approval)
        .map(|n| n.id.clone())
        .collect();

    // ---- 8. 孤立节点（warning）：无入边也无出边，不参与图主流程 ----
    // （DAG 中非入口节点必可达；真正需要提示的是“悬空”节点）
    let has_incoming: BTreeSet<&str> = edges.iter().map(|e| e.to.as_str()).collect();
    let has_outgoing: BTreeSet<&str> = edges.iter().map(|e| e.from.as_str()).collect();
    if spec.nodes.len() > 1 {
        for n in &spec.nodes {
            if !has_incoming.contains(n.id.as_str()) && !has_outgoing.contains(n.id.as_str()) {
                warnings.push(issue(
                    "unreachable_node",
                    format!("节点 '{}' 孤立（无入边也无出边），不参与图主流程", n.id),
                    Some(n.id.clone()),
                ));
            }
        }
    }

    let compiled = CompiledGraph {
        goal: spec.goal.clone(),
        repository: spec.repository.clone(),
        base_revision: spec.base_revision.clone(),
        integration_target: spec.integration_target.clone(),
        max_concurrency: spec.max_concurrency,
        nodes: spec.nodes.clone(),
        edges,
        execution_order: order,
        parallel_groups,
        required_approvals,
        resource_conflicts: conflicts,
        conflict_pairs,
    };

    finish(spec, errors, warnings).map_valid(compiled)
}

fn finish(
    spec: &GraphSpec,
    errors: Vec<CompileIssue>,
    warnings: Vec<CompileIssue>,
) -> CompileResult {
    let _ = spec;
    CompileResult {
        valid: errors.is_empty(),
        errors,
        warnings,
        compiled: None,
    }
}

trait MapValid {
    fn map_valid(self, compiled: CompiledGraph) -> CompileResult;
}

impl MapValid for CompileResult {
    fn map_valid(mut self, compiled: CompiledGraph) -> CompileResult {
        self.compiled = Some(compiled);
        self
    }
}

fn issue(code: &'static str, message: impl Into<String>, node: Option<String>) -> CompileIssue {
    CompileIssue {
        code,
        message: message.into(),
        node,
    }
}

/// 节点级契约校验（docs/design_graph.md §三/§十二）。
fn contract_check(n: &NodeSpec, errors: &mut Vec<CompileIssue>, warnings: &mut Vec<CompileIssue>) {
    let id = n.id.clone();

    if n.timeout_seconds == 0 {
        errors.push(issue(
            "timeout_required",
            format!("节点 '{id}' 必须声明 > 0 的超时（timeout_seconds）"),
            Some(id.clone()),
        ));
    }

    match n.node_type {
        NodeType::Executor | NodeType::Deterministic => {
            if n.outputs.schema.trim().is_empty() {
                errors.push(issue(
                    "output_schema_required",
                    format!("节点 '{id}' 必须声明输出 Schema（outputs.schema）"),
                    Some(id.clone()),
                ));
            }
            if n.acceptance.is_empty() {
                errors.push(issue(
                    "acceptance_required",
                    format!("节点 '{id}' 必须具备验收规则（acceptance）"),
                    Some(id.clone()),
                ));
            }
            let is_writer = !n.write_paths.is_empty();
            if is_writer && n.workspace.is_none() {
                errors.push(issue(
                    "workspace_required",
                    format!("写节点 '{id}' 必须绑定 workspace"),
                    Some(id.clone()),
                ));
            }
        }
        NodeType::Approval => match &n.approval {
            Some(a) if !a.message.trim().is_empty() => {}
            _ => errors.push(issue(
                "approval_message_required",
                format!("approval 节点 '{id}' 必须声明 approval.message"),
                Some(id.clone()),
            )),
        },
        NodeType::Join => {
            if n.join_policy.is_none() {
                errors.push(issue(
                    "join_policy_required",
                    format!("join 节点 '{id}' 必须声明 join_policy"),
                    Some(id.clone()),
                ));
            }
            if n.dependencies.is_empty() {
                errors.push(issue(
                    "join_needs_upstream",
                    format!("join 节点 '{id}' 至少需要一个上游依赖"),
                    Some(id.clone()),
                ));
            }
        }
        NodeType::Gate => {
            // 第一版 gate_condition 可选（M1 语义）；目标节点存在性已由边检查覆盖。
        }
    }

    // Executor 特有契约：participant 必填（Deterministic 允许无 participant，M1 扩展 Runtime 执行）。
    if n.node_type == NodeType::Executor && n.executor_requirements.participant.is_none() {
        errors.push(issue(
            "participant_required",
            format!("executor 节点 '{id}' 必须声明 executor_requirements.participant"),
            Some(id.clone()),
        ));
    }

    // 安全：写节点建议声明 forbidden_paths
    if !n.write_paths.is_empty() && n.forbidden_paths.is_empty() {
        warnings.push(issue(
            "forbidden_paths_recommended",
            format!("写节点 '{id}' 未声明 forbidden_paths（建议声明防越界）"),
            Some(id.clone()),
        ));
    }
    // 安全：executor 无任何路径声明 = 无边界文件系统访问
    if n.node_type == NodeType::Executor && n.read_paths.is_empty() && n.write_paths.is_empty() {
        warnings.push(issue(
            "unbounded_fs_access",
            format!("executor 节点 '{id}' 未声明 read/write_paths（无边界文件系统访问）"),
            Some(id.clone()),
        ));
    }
    // 安全：无限重试防护
    if n.retry_policy.max_attempts > 20 {
        warnings.push(issue(
            "unbounded_retries",
            format!(
                "节点 '{id}' 的 retry_policy.max_attempts={} 过大（疑似无限重试）",
                n.retry_policy.max_attempts
            ),
            Some(id.clone()),
        ));
    }
}

/// 三色 DFS 环检测，返回环路径（如有）。
fn find_cycle<'a>(
    nodes: &'a [NodeSpec],
    adj: &BTreeMap<&'a str, Vec<&'a str>>,
) -> Option<Vec<String>> {
    fn dfs<'a>(
        node: &'a str,
        adj: &BTreeMap<&'a str, Vec<&'a str>>,
        color: &mut BTreeMap<&'a str, u8>,
        stack: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        color.insert(node, 1);
        stack.push(node.to_string());
        if let Some(nexts) = adj.get(node) {
            for next in nexts {
                match color.get(*next) {
                    Some(2) => continue,
                    Some(1) => {
                        let start = stack.iter().position(|s| s.as_str() == *next)?;
                        let mut cycle: Vec<String> = stack[start..].to_vec();
                        cycle.push((*next).to_string());
                        return Some(cycle);
                    }
                    _ => {
                        if let Some(c) = dfs(next, adj, color, stack) {
                            return Some(c);
                        }
                    }
                }
            }
        }
        stack.pop();
        color.insert(node, 2);
        None
    }

    let mut color: BTreeMap<&str, u8> = BTreeMap::new();
    for n in nodes {
        if !color.contains_key(n.id.as_str()) {
            let mut stack = Vec::new();
            if let Some(c) = dfs(&n.id, adj, &mut color, &mut stack) {
                return Some(c);
            }
        }
    }
    None
}

/// Kahn 拓扑排序（on_success 依赖）；环已在前置检查拒绝，此处不会卡死。
fn topo_order(nodes: &[NodeSpec]) -> Vec<String> {
    let mut indegree: BTreeMap<&str, usize> = nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for n in nodes {
        for d in &n.dependencies {
            if indegree.contains_key(d.as_str()) {
                *indegree.entry(&n.id).or_default() += 1;
                children.entry(d.as_str()).or_default().push(&n.id);
            }
        }
    }
    let mut queue: Vec<&str> = indegree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(&k, _)| k)
        .collect();
    let mut order = Vec::new();
    while let Some(node) = queue.pop() {
        order.push(node.to_string());
        if let Some(cs) = children.get(node) {
            for c in cs {
                let d = indegree.get_mut(*c).unwrap();
                *d -= 1;
                if *d == 0 {
                    queue.push(*c);
                }
            }
        }
    }
    // 有环时可能不完整（前置已拒绝，这里只是兜底）
    order
}

/// 拓扑层分组（同层节点无依赖，是并行候选）。
fn topo_levels(nodes: &[NodeSpec]) -> Vec<Vec<String>> {
    let mut indegree: BTreeMap<&str, usize> = nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for n in nodes {
        for d in &n.dependencies {
            if indegree.contains_key(d.as_str()) {
                *indegree.entry(&n.id).or_default() += 1;
                children.entry(d.as_str()).or_default().push(&n.id);
            }
        }
    }
    let mut levels = Vec::new();
    loop {
        let ready: Vec<&str> = indegree
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(&k, _)| k)
            .collect();
        if ready.is_empty() {
            break;
        }
        for r in &ready {
            indegree.remove(*r);
        }
        let mut level: Vec<String> = ready.iter().map(|s| s.to_string()).collect();
        level.sort();
        for r in ready {
            if let Some(cs) = children.get(r) {
                for c in cs {
                    if let Some(d) = indegree.get_mut(*c) {
                        *d -= 1;
                    }
                }
            }
        }
        levels.push(level);
    }
    levels
}

/// 并行冲突校验：write_paths 相交 / 共享契约文件 / 同一 workspace key 的写节点互斥。
/// 返回 (冲突描述, 额外 warning)。冲突只影响并行调度（M1），不阻塞编译（M0 串行）。
fn parallel_conflicts(spec: &GraphSpec) -> (Vec<(String, String)>, Vec<String>, Vec<CompileIssue>) {
    let writers: Vec<&NodeSpec> = spec
        .nodes
        .iter()
        .filter(|n| !n.write_paths.is_empty())
        .collect();
    let mut conflict_pairs: Vec<(String, String)> = Vec::new();
    let mut conflicts = Vec::new();
    let mut warnings = Vec::new();

    for (i, a) in writers.iter().enumerate() {
        for b in writers.iter().skip(i + 1) {
            let a_paths = &a.write_paths;
            let b_paths = &b.write_paths;
            let overlap = a_paths
                .iter()
                .any(|pa| b_paths.iter().any(|pb| paths_overlap(pa, pb)));
            if overlap {
                conflict_pairs.push((a.id.clone(), b.id.clone()));
                conflicts.push(format!(
                    "写节点 '{}' 与 '{}' 的 write_paths 重叠，不能并行",
                    a.id, b.id
                ));
            }
            let shared = a_paths.iter().any(|pa| {
                b_paths
                    .iter()
                    .any(|pb| touches_shared_contract(pa) || touches_shared_contract(pb))
            });
            if shared {
                conflict_pairs.push((a.id.clone(), b.id.clone()));
                conflicts.push(format!(
                    "写节点 '{}' 与 '{}' 触碰共享契约文件（锁文件/迁移），不能并行",
                    a.id, b.id
                ));
            }
            if let (Some(wa), Some(wb)) = (&a.workspace, &b.workspace) {
                if wa == wb {
                    conflict_pairs.push((a.id.clone(), b.id.clone()));
                    conflicts.push(format!(
                        "写节点 '{}' 与 '{}' 使用同一 workspace '{}'，不能并行",
                        a.id, b.id, wa
                    ));
                }
            }
        }
    }

    // 写节点间无重叠但共享契约的跨层情形已由 touches_shared_contract 覆盖；
    // 这里补一个全局提示：任何触碰共享契约文件的写节点都应在调度层串行。
    let contract_touchers: Vec<&str> = writers
        .iter()
        .filter(|n| n.write_paths.iter().any(|p| touches_shared_contract(p)))
        .map(|n| n.id.as_str())
        .collect();
    if contract_touchers.len() > 1 {
        warnings.push(issue(
            "shared_contract_serialized",
            format!(
                "触碰共享契约文件的写节点（{}）调度时默认串行",
                contract_touchers.join(", ")
            ),
            None,
        ));
    }

    (conflict_pairs, conflicts, warnings)
}
