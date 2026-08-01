//! 图工程运行时（docs/design_graph.md §5.1）。
//!
//! M0：spec 解析（spec.rs）+ Graph Compiler（compiler.rs，路径工具 paths.rs）。
//! M1 起加入 state / scheduler / workspace / artifact / verify / events / reconciler。

pub mod compiler;
mod paths;
pub mod spec;
#[cfg(test)]
pub mod tests;

pub use compiler::{CompileIssue, CompileResult, CompiledGraph, Edge, Trigger};
pub use spec::{
    AcceptanceRule, ApprovalSpec, ExecutorRequirements, FailureType, GraphSpec, JoinPolicy,
    NodeSpec, NodeType, OutputSpec, RetryPolicy,
};

/// 图工程领域错误（thiserror；daemon 边界统一转 ServerMsg::Error）。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("graph spec 解析失败: {0}")]
    SpecParse(#[from] serde_yaml::Error),
    #[error("graph spec 校验未通过: {} 个错误", .0)]
    CompileFailed(usize),
}
