//! Graph Spec 解析与 Typed Node 契约（docs/design_graph.md §5.1 spec.rs）。
//!
//! 只负责 YAML → 结构化契约的解析，不包含任何校验逻辑（校验在 compiler.rs）。
//! 必填字段在 serde 层强制（缺字段 → 解析错误），语义约束（如写节点必须绑 workspace）
//! 在 compiler 层校验。

use serde::Deserialize;

/// 节点 ID（图中唯一，路由/依赖引用键）。
pub type NodeKey = String;

/// 第一版五类基础节点（docs/design_graph.md §三）。
/// P0 运行时只实现 executor / deterministic / approval；join / gate 结构校验先行（P1 生效）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    Executor,
    Deterministic,
    Join,
    Gate,
    Approval,
}

impl NodeType {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeType::Executor => "executor",
            NodeType::Deterministic => "deterministic",
            NodeType::Join => "join",
            NodeType::Gate => "gate",
            NodeType::Approval => "approval",
        }
    }

    pub fn from_str_name(s: &str) -> Option<Self> {
        match s {
            "executor" => Some(NodeType::Executor),
            "deterministic" => Some(NodeType::Deterministic),
            "join" => Some(NodeType::Join),
            "gate" => Some(NodeType::Gate),
            "approval" => Some(NodeType::Approval),
            _ => None,
        }
    }
}

/// Join 汇聚策略（第一版只支持两种，见 docs/design_graph.md §四）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinPolicy {
    AllSucceeded,
    AllTerminal,
}

/// 失败分类（docs/design_graph.md §八）。重试策略按分类判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureType {
    ExecutionError,
    ValidationError,
    TestFailure,
    BuildFailure,
    PathViolation,
    ContractViolation,
    AgentTimeout,
    AgentUnavailable,
    WorkspaceFailure,
    MergeConflict,
    SemanticBlocker,
    ApprovalRejected,
}

impl FailureType {
    pub fn as_str(self) -> &'static str {
        match self {
            FailureType::ExecutionError => "execution_error",
            FailureType::ValidationError => "validation_error",
            FailureType::TestFailure => "test_failure",
            FailureType::BuildFailure => "build_failure",
            FailureType::PathViolation => "path_violation",
            FailureType::ContractViolation => "contract_violation",
            FailureType::AgentTimeout => "agent_timeout",
            FailureType::AgentUnavailable => "agent_unavailable",
            FailureType::WorkspaceFailure => "workspace_failure",
            FailureType::MergeConflict => "merge_conflict",
            FailureType::SemanticBlocker => "semantic_blocker",
            FailureType::ApprovalRejected => "approval_rejected",
        }
    }
}

const fn default_max_concurrency() -> u32 {
    1
}

const fn default_max_attempts() -> u32 {
    1
}

/// 重试策略。无无限重试：max_attempts 必填语义由默认值 1 兜底。
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct RetryPolicy {
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
    #[serde(default)]
    pub backoff_seconds: u32,
    /// 允许重试的失败分类；空 = 不可重试。
    #[serde(default)]
    pub retryable: Vec<FailureType>,
}

/// 节点输出契约。
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct OutputSpec {
    /// 输出 Schema 名（compiler 校验非空；schema 内容语义校验在 M2 运行时）。
    pub schema: String,
    /// 声明产出物名称，供下游按名引用（Artifact 引用语义在 M1/M2）。
    #[serde(default)]
    pub artifacts: Vec<String>,
}

/// 执行者能力要求（M1 调度按 participant / capabilities 匹配）。
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ExecutorRequirements {
    #[serde(default)]
    pub participant: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// 验收规则（M2 Verification 消费）。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AcceptanceRule {
    /// path / schema / artifact / command / review
    #[serde(rename = "type")]
    pub rule_type: String,
    #[serde(default)]
    pub rule: String,
}

/// Approval 节点配置（复用 human approval 仲裁，docs/design_graph.md §5.2 取舍 3）。
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ApprovalSpec {
    pub message: String,
    #[serde(default)]
    pub options: Vec<String>,
}

/// 单个节点的契约（docs/design_graph.md §三 Typed Node 契约）。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NodeSpec {
    pub id: NodeKey,
    #[serde(rename = "type")]
    pub node_type: NodeType,
    /// on_success 前置依赖（执行顺序与环检测的基础）。
    #[serde(default)]
    pub dependencies: Vec<NodeKey>,
    /// 失败触发边。
    #[serde(default)]
    pub on_failure: Vec<NodeKey>,
    /// 阻塞触发边。
    #[serde(default)]
    pub on_blocked: Vec<NodeKey>,
    /// 无条件触发边。
    #[serde(default)]
    pub always: Vec<NodeKey>,
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub outputs: OutputSpec,
    #[serde(default)]
    pub executor_requirements: ExecutorRequirements,
    /// 写入节点必须绑定的 workspace key（compiler 校验）。
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub read_paths: Vec<String>,
    #[serde(default)]
    pub write_paths: Vec<String>,
    #[serde(default)]
    pub forbidden_paths: Vec<String>,
    #[serde(default)]
    pub acceptance: Vec<AcceptanceRule>,
    /// 超时秒数（必填——每个节点必须声明超时）。
    pub timeout_seconds: u32,
    #[serde(default)]
    pub retry_policy: RetryPolicy,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub approval: Option<ApprovalSpec>,
    #[serde(default)]
    pub join_policy: Option<JoinPolicy>,
    #[serde(default)]
    pub gate_condition: Option<String>,
    #[serde(default)]
    pub estimated_duration: Option<f64>,
}

/// Graph Spec（docs/design_graph.md §三 图目标/节点/依赖/并发/重试/验收）。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GraphSpec {
    /// 当前必须为 1（compiler 校验）。
    pub version: u32,
    pub goal: String,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub base_revision: Option<String>,
    #[serde(default)]
    pub integration_target: Option<String>,
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: u32,
    #[serde(default)]
    pub nodes: Vec<NodeSpec>,
}

impl GraphSpec {
    /// 解析 YAML 文本为 GraphSpec。语法错误 → Err；缺必填字段 → Err（serde）。
    pub fn parse(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }
}
