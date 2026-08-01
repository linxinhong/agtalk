//! 图工程运行时（docs/design_graph.md §5.1）。
//!
//! M0：spec 解析（spec.rs）+ Graph Compiler（compiler.rs，路径工具 paths.rs）。
//! M1：NodeRun/GraphRun 状态机（state.rs）+ GraphEvent（events.rs）。

pub mod analyze;
pub mod approval;
pub mod compiler;
pub mod dto;
pub mod events;
pub mod paths;
pub mod reconciler;
pub mod scheduler;
pub mod spec;
pub mod state;
#[cfg(test)]
pub mod state_tests;
#[cfg(test)]
pub mod tests;
pub mod verify;
pub mod workspace;

pub use compiler::{CompileIssue, CompileResult, CompiledGraph, Edge, Trigger};
pub use scheduler::{tick, DispatchItem};
pub use spec::{
    AcceptanceRule, ApprovalSpec, ExecutorRequirements, FailureType, GraphSpec, JoinPolicy,
    NodeSpec, NodeType, OutputSpec, RetryPolicy,
};
pub use state::{
    can_transition, converge_graph_run, create_node_run, get_node_run, get_node_run_by_key,
    list_node_runs, transition, GraphRunStatus, NodeRunRow, NodeRunStatus, TransitionOutcome,
};

/// 图工程领域错误（thiserror；daemon 边界统一转 ServerMsg::Error）。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("graph spec 解析失败: {0}")]
    SpecParse(#[from] serde_yaml::Error),
    #[error("graph spec 校验未通过: {} 个错误", .0)]
    CompileFailed(usize),
    #[error("SQLite 错误: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("JSON 错误: {0}")]
    Json(#[from] serde_json::Error),
}
