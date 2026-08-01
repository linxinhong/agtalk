//! 图工程协议 DTO（docs/design_graph.md §7；proto.rs 只放 enum 变体，DTO 内聚于此）。

use serde::{Deserialize, Serialize};

/// 节点产物引用（docs/graph-participant-protocol.md §6 result.json）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphArtifactRef {
    #[serde(default)]
    pub artifact_type: String,
    #[serde(default)]
    pub schema_version: String,
    #[serde(default)]
    pub uri: String,
    #[serde(default)]
    pub checksum: String,
}

/// 自证验证项（P0 信任 + 落库留证；M2 起 daemon 抽查）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphClaim {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub exit_code: i32,
    #[serde(default)]
    pub stdout_ref: String,
    #[serde(default)]
    pub summary: String,
}

/// 编译诊断项（协议视图，供 GUI/CLI 展示）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphCompileIssue {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
}

/// GraphRun 摘要（列表项 / 详情头）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphRunSummary {
    pub id: String,
    pub goal: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<String>,
    pub created_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
}

/// NodeRun 协议视图。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNodeDetail {
    pub node_key: String,
    pub node_type: String,
    pub status: String,
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_detail: Option<String>,
}

/// GraphEvent 协议视图（SSE/events 端点）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEventDto {
    pub id: i64,
    pub event_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_key: Option<String>,
    pub payload: serde_json::Value,
    pub created_at: f64,
}
