//! 图成本分析（docs/graph-engineering-survey.md §5 决策规则的代码化）。
//!
//! `analyze` 纯本地评估（不建图、不调 daemon）：解析 + 编译 + 统计
//! （节点数/最长链/并行度/写节点/验证节点/审批节点/时长下界），
//! 按"至少一条值得上图"规则给出 verdict。

use crate::graph::compiler::CompiledGraph;

/// 评估结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnalysisVerdict {
    /// 至少命中一条值得上图条件。
    WorthIt,
    /// 简单任务，不值得上图（走单 agent）。
    NotWorthIt,
}

/// 图成本分析结果。
#[derive(Debug, Clone)]
pub struct GraphAnalysis {
    pub node_count: usize,
    pub longest_chain: usize,
    pub parallel_factor: usize, // 首层就绪节点数（可并行度代理）
    pub writer_nodes: usize,
    pub verification_nodes: usize,
    pub approval_nodes: usize,
    pub est_duration_secs: f64, // 最长链上 timeout 之和（乐观串行下界）
    pub verdict: AnalysisVerdict,
    pub reasons: Vec<String>,
}

/// 评估一张已编译的图（survey §5 决策规则）。
pub fn analyze(compiled: &CompiledGraph) -> GraphAnalysis {
    // 拓扑 DP 求最长依赖链深度（DAG 已由 compiler 保证无环）
    let mut depth: std::collections::BTreeMap<&str, usize> = Default::default();
    for node in &compiled.execution_order {
        let dep_max = compiled
            .nodes
            .iter()
            .find(|n| n.id == *node)
            .map(|n| {
                n.dependencies
                    .iter()
                    .filter_map(|d| depth.get(d.as_str()))
                    .copied()
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        depth.insert(node.as_str(), dep_max + 1);
    }
    let longest_chain = depth.values().copied().max().unwrap_or(0);

    // 首层就绪节点（无 on_success 依赖）——可并行度代理
    let first_layer = compiled
        .execution_order
        .iter()
        .filter(|n| {
            compiled
                .nodes
                .iter()
                .find(|sn| sn.id == **n)
                .map(|sn| sn.dependencies.is_empty())
                .unwrap_or(false)
        })
        .count();

    // 节点分类
    let writer_nodes = compiled
        .nodes
        .iter()
        .filter(|n| !n.write_paths.is_empty())
        .count();
    let verification_nodes = compiled
        .nodes
        .iter()
        .filter(|n| !n.acceptance.is_empty())
        .count();
    let approval_nodes = compiled
        .nodes
        .iter()
        .filter(|n| n.node_type == crate::graph::spec::NodeType::Approval)
        .count();

    // 时长下界：沿最大深度依赖回溯最长链，各节点 timeout 之和
    let mut est_duration_secs: f64 = 0.0;
    let mut cur_node: Option<&str> = compiled
        .execution_order
        .iter()
        .max_by_key(|n| depth.get(n.as_str()).copied().unwrap_or(0))
        .map(|n| n.as_str());
    while let Some(id) = cur_node {
        let Some(sn) = compiled.nodes.iter().find(|n| n.id == id) else {
            break;
        };
        est_duration_secs += sn.timeout_seconds as f64;
        cur_node = sn
            .dependencies
            .iter()
            .max_by_key(|d| depth.get(d.as_str()).copied().unwrap_or(0))
            .map(|d| d.as_str());
    }

    // 判定（survey §5：至少一条值得 → WorthIt）
    let mut reasons = Vec::new();
    if first_layer >= 3 {
        reasons.push(format!("可并行度≥3（首层 {first_layer} 个就绪节点）"));
    }
    if est_duration_secs > 1800.0 {
        reasons.push(format!(
            "预估时长>{:.0} 分钟（最长链下界 {:.0}s）",
            est_duration_secs / 60.0,
            est_duration_secs
        ));
    }
    // 验证条件需图规模≥3：单节点自检（acceptance）只是节点自身验收，无汇聚/门禁价值
    if verification_nodes > 0 && compiled.nodes.len() >= 3 {
        reasons.push(format!(
            "验证可自动化（{verification_nodes} 个 acceptance 节点）"
        ));
    }
    if approval_nodes > 0 {
        reasons.push(format!(
            "有人类审批门禁（{approval_nodes} 个 approval 节点）"
        ));
    }
    let verdict = if reasons.is_empty() {
        AnalysisVerdict::NotWorthIt
    } else {
        AnalysisVerdict::WorthIt
    };

    GraphAnalysis {
        node_count: compiled.nodes.len(),
        longest_chain,
        parallel_factor: first_layer,
        writer_nodes,
        verification_nodes,
        approval_nodes,
        est_duration_secs,
        verdict,
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::spec::GraphSpec;

    fn analyze_yaml(yaml: &str) -> GraphAnalysis {
        let spec = GraphSpec::parse(yaml).expect("spec 解析失败");
        let compiled = crate::graph::compiler::compile(&spec);
        assert!(compiled.valid, "spec 编译失败: {:?}", compiled.errors);
        analyze(compiled.compiled.as_ref().expect("编译产物缺失"))
    }

    #[test]
    fn parallel_graph_is_worth_it() {
        // 3 个并行写节点 + join → 命中可并行度条件
        let yaml = r#"
version: 1
goal: "parallel"
nodes:
  - id: a
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p1 }
    workspace: w1
    write_paths: [src/a]
    acceptance: [{ type: path }]
    timeout_seconds: 600
  - id: b
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p2 }
    workspace: w2
    write_paths: [src/b]
    acceptance: [{ type: path }]
    timeout_seconds: 600
  - id: c
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p3 }
    workspace: w3
    write_paths: [src/c]
    acceptance: [{ type: path }]
    timeout_seconds: 600
  - id: join
    type: join
    dependencies: [a, b, c]
    join_policy: all_succeeded
    timeout_seconds: 60
"#;
        let a = analyze_yaml(yaml);
        assert_eq!(a.verdict, AnalysisVerdict::WorthIt);
        assert_eq!(a.parallel_factor, 3);
        assert_eq!(a.longest_chain, 2);
        assert_eq!(a.est_duration_secs, 660.0);
    }

    #[test]
    fn single_simple_node_not_worth_it() {
        let yaml = r#"
version: 1
goal: "tiny"
nodes:
  - id: a
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p1 }
    workspace: w1
    acceptance: [{ type: path }]
    timeout_seconds: 60
"#;
        let a = analyze_yaml(yaml);
        assert_eq!(a.verdict, AnalysisVerdict::NotWorthIt, "简单任务不值得上图");
        assert!(a.reasons.is_empty());
    }

    #[test]
    fn approval_or_verification_triggers_worth() {
        let yaml = r#"
version: 1
goal: "gate"
nodes:
  - id: a
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p1 }
    workspace: w1
    acceptance: [{ type: path }]
    timeout_seconds: 60
  - id: ap
    type: approval
    dependencies: [a]
    approval:
      message: "确认合并？"
      options: [yes, no]
    timeout_seconds: 300
"#;
        let a = analyze_yaml(yaml);
        assert_eq!(a.verdict, AnalysisVerdict::WorthIt, "有人类审批门禁 → 值得");
        assert!(a.reasons.iter().any(|r| r.contains("审批")));
    }

    #[test]
    fn long_task_worth_it_by_duration() {
        let yaml = r#"
version: 1
goal: "long"
nodes:
  - id: a
    type: executor
    outputs: { schema: s }
    executor_requirements: { participant: p1 }
    workspace: w1
    acceptance: [{ type: path }]
    timeout_seconds: 2400
"#;
        let a = analyze_yaml(yaml);
        assert_eq!(a.verdict, AnalysisVerdict::WorthIt, "时长>30min → 值得");
        assert!(a.reasons.iter().any(|r| r.contains("时长")));
    }
}
