//! NodeRun / GraphRun 状态机（docs/design_graph.md §5.3 / §5.4）。
//!
//! 原则：状态推进只发生在 daemon 内（Scheduler / Runtime / Verifier / Reconciler 调用
//! transition），Agent 只能"上报"，永远不能直接改状态。所有迁移写入 graph_events 审计。

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::spec::NodeType;

/// 节点 ID（图内唯一）。
pub type NodeKey = String;

/// NodeRun 状态（design_graph.md §5.3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRunStatus {
    Pending,
    Ready,
    Leased,
    Dispatched,
    Running,
    Verifying,
    /// Approval 节点等待人工授权（design_graph.md §9 决策：独立状态，GUI 黄灯）。
    WaitingApproval,
    Succeeded,
    Failed,
    Blocked,
    TimedOut,
    Cancelled,
}

impl NodeRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeRunStatus::Pending => "pending",
            NodeRunStatus::Ready => "ready",
            NodeRunStatus::Leased => "leased",
            NodeRunStatus::Dispatched => "dispatched",
            NodeRunStatus::Running => "running",
            NodeRunStatus::Verifying => "verifying",
            NodeRunStatus::WaitingApproval => "waiting_approval",
            NodeRunStatus::Succeeded => "succeeded",
            NodeRunStatus::Failed => "failed",
            NodeRunStatus::Blocked => "blocked",
            NodeRunStatus::TimedOut => "timed_out",
            NodeRunStatus::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            NodeRunStatus::Succeeded
                | NodeRunStatus::Failed
                | NodeRunStatus::Blocked
                | NodeRunStatus::TimedOut
                | NodeRunStatus::Cancelled
        )
    }

    /// 活动状态：已被调度占用或正在执行/等待，不能再次派发。
    pub fn is_active(self) -> bool {
        !self.is_terminal() && !matches!(self, NodeRunStatus::Pending | NodeRunStatus::Ready)
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(NodeRunStatus::Pending),
            "ready" => Some(NodeRunStatus::Ready),
            "leased" => Some(NodeRunStatus::Leased),
            "dispatched" => Some(NodeRunStatus::Dispatched),
            "running" => Some(NodeRunStatus::Running),
            "verifying" => Some(NodeRunStatus::Verifying),
            "waiting_approval" => Some(NodeRunStatus::WaitingApproval),
            "succeeded" => Some(NodeRunStatus::Succeeded),
            "failed" => Some(NodeRunStatus::Failed),
            "blocked" => Some(NodeRunStatus::Blocked),
            "timed_out" => Some(NodeRunStatus::TimedOut),
            "cancelled" => Some(NodeRunStatus::Cancelled),
            _ => None,
        }
    }
}

/// GraphRun 状态（design_graph.md §5.4）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphRunStatus {
    Draft,
    Validating,
    Ready,
    Running,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl GraphRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            GraphRunStatus::Draft => "draft",
            GraphRunStatus::Validating => "validating",
            GraphRunStatus::Ready => "ready",
            GraphRunStatus::Running => "running",
            GraphRunStatus::Paused => "paused",
            GraphRunStatus::Completed => "completed",
            GraphRunStatus::Failed => "failed",
            GraphRunStatus::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            GraphRunStatus::Completed | GraphRunStatus::Failed | GraphRunStatus::Cancelled
        )
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(GraphRunStatus::Draft),
            "validating" => Some(GraphRunStatus::Validating),
            "ready" => Some(GraphRunStatus::Ready),
            "running" => Some(GraphRunStatus::Running),
            "paused" => Some(GraphRunStatus::Paused),
            "completed" => Some(GraphRunStatus::Completed),
            "failed" => Some(GraphRunStatus::Failed),
            "cancelled" => Some(GraphRunStatus::Cancelled),
            _ => None,
        }
    }
}

/// NodeRun 状态迁移表（谁推进 → 什么迁移合法）。
/// 不变量：Agent 上报只触发 running/verifying/failed 等"结果"迁移；
/// 调度类迁移（ready/leased/dispatched/waiting_approval）只由 Scheduler 发起。
pub fn can_transition(from: NodeRunStatus, to: NodeRunStatus) -> bool {
    use NodeRunStatus::*;
    matches!(
        (from, to),
        (Pending, Ready)
            | (Pending, Blocked)
            | (Pending, Cancelled)
            | (Ready, Leased)
            | (Ready, Blocked)
            | (Ready, Cancelled)
            | (Leased, Dispatched)
            | (Leased, TimedOut)
            | (Leased, Cancelled)
            | (Dispatched, Running)
            | (Dispatched, WaitingApproval)
            | (Dispatched, Succeeded)
            | (Dispatched, TimedOut)
            | (Dispatched, Cancelled)
            | (Running, Verifying)
            | (Running, TimedOut)
            | (Running, Blocked)
            | (Running, WaitingApproval)
            | (Running, Cancelled)
            | (WaitingApproval, Running)
            | (WaitingApproval, Succeeded)
            | (WaitingApproval, Failed)
            | (WaitingApproval, Cancelled)
            | (Verifying, Succeeded)
            | (Verifying, Failed)
            | (Verifying, Cancelled) // 终态不迁移（failed → 新 attempt 是"新行"不是迁移）
    )
}

/// NodeRun 数据库行（读模型）。
#[derive(Debug, Clone, PartialEq)]
pub struct NodeRunRow {
    pub id: String,
    pub graph_run_id: String,
    pub node_key: NodeKey,
    pub node_type: String,
    pub status: NodeRunStatus,
    pub attempt: u32,
    pub participant_id: Option<String>,
    pub workspace_id: Option<String>,
    pub lease_token: Option<String>,
    pub lease_expires_at: Option<f64>,
    pub started_at: Option<f64>,
    pub heartbeat_at: Option<f64>,
    pub completed_at: Option<f64>,
    pub failure_type: Option<String>,
    pub failure_detail: Option<String>,
    pub version: u32,
}

/// 状态推进结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionOutcome {
    Applied,
    /// 乐观锁冲突：其他推进者已先改状态（重读后重试或放弃）。
    VersionConflict,
    /// 非法迁移（状态机不允许）。
    IllegalTransition,
}

/// 乐观锁状态推进：`UPDATE ... SET status=? WHERE id=? AND version=?`。
/// 成功时 version+1，并写 graph_events 审计（先落库后推送，design_graph.md §11 红线 6）。
/// 参数多是因为迁移事件字段天然内聚（from/to/failure/心跳），合并反而损害可读性。
#[allow(clippy::too_many_arguments)]
pub fn transition(
    conn: &Connection,
    node_run_id: &str,
    expected_version: u32,
    from: NodeRunStatus,
    to: NodeRunStatus,
    failure_type: Option<&str>,
    failure_detail: Option<&str>,
    heartbeat_at: Option<f64>,
) -> Result<TransitionOutcome, super::Error> {
    if !can_transition(from, to) {
        return Ok(TransitionOutcome::IllegalTransition);
    }
    let changed = conn
        .execute(
            "UPDATE node_runs SET status=?1, version=version+1, \
             completed_at=CASE WHEN ?2 THEN ?3 ELSE completed_at END, \
             failure_type=?4, failure_detail=?5, \
             heartbeat_at=CASE WHEN ?6 IS NOT NULL THEN ?6 ELSE heartbeat_at END \
             WHERE id=?7 AND version=?8",
            params![
                to.as_str(),
                to.is_terminal(),
                unix_now(),
                failure_type,
                failure_detail,
                heartbeat_at,
                node_run_id,
                expected_version,
            ],
        )
        .map_err(super::Error::Sqlite)?;

    if changed == 0 {
        // 行不存在或版本不匹配 → 乐观锁冲突
        let exists: Option<String> = conn
            .query_row(
                "SELECT id FROM node_runs WHERE id=?1",
                params![node_run_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(super::Error::Sqlite)?;
        return Ok(if exists.is_some() {
            TransitionOutcome::VersionConflict
        } else {
            TransitionOutcome::IllegalTransition
        });
    }

    // 审计事件（graph_run_id 需要反查）
    let graph_run_id: String = conn
        .query_row(
            "SELECT graph_run_id FROM node_runs WHERE id=?1",
            params![node_run_id],
            |r| r.get(0),
        )
        .map_err(super::Error::Sqlite)?;
    let node_key: String = conn
        .query_row(
            "SELECT node_key FROM node_runs WHERE id=?1",
            params![node_run_id],
            |r| r.get(0),
        )
        .map_err(super::Error::Sqlite)?;
    super::events::append(
        conn,
        &graph_run_id,
        &format!("node_{}", to.as_str()),
        Some(&node_key),
        &serde_json::json!({ "from": from.as_str(), "to": to.as_str(), "attempt": 0 }),
    )?;
    Ok(TransitionOutcome::Applied)
}

/// 读取单个 NodeRun。
pub fn get_node_run(
    conn: &Connection,
    node_run_id: &str,
) -> Result<Option<NodeRunRow>, super::Error> {
    conn.query_row(
        "SELECT id, graph_run_id, node_key, node_type, status, attempt, participant_id, \
         workspace_id, lease_token, lease_expires_at, started_at, heartbeat_at, completed_at, \
         failure_type, failure_detail, version \
         FROM node_runs WHERE id=?1",
        params![node_run_id],
        |r| {
            Ok(NodeRunRow {
                id: r.get(0)?,
                graph_run_id: r.get(1)?,
                node_key: r.get(2)?,
                node_type: r.get(3)?,
                status: NodeRunStatus::parse_str(&r.get::<_, String>(4)?)
                    .unwrap_or(NodeRunStatus::Pending),
                attempt: r.get(5)?,
                participant_id: r.get(6)?,
                workspace_id: r.get(7)?,
                lease_token: r.get(8)?,
                lease_expires_at: r.get(9)?,
                started_at: r.get(10)?,
                heartbeat_at: r.get(11)?,
                completed_at: r.get(12)?,
                failure_type: r.get(13)?,
                failure_detail: r.get(14)?,
                version: r.get(15)?,
            })
        },
    )
    .optional()
    .map_err(super::Error::Sqlite)
}

/// 按幂等键读取 NodeRun（graph_run_id + node_key + attempt）。
pub fn get_node_run_by_key(
    conn: &Connection,
    graph_run_id: &str,
    node_key: &str,
    attempt: u32,
) -> Result<Option<NodeRunRow>, super::Error> {
    conn.query_row(
        "SELECT id, graph_run_id, node_key, node_type, status, attempt, participant_id, \
         workspace_id, lease_token, lease_expires_at, started_at, heartbeat_at, completed_at, \
         failure_type, failure_detail, version \
         FROM node_runs WHERE graph_run_id=?1 AND node_key=?2 AND attempt=?3",
        params![graph_run_id, node_key, attempt],
        |r| {
            Ok(NodeRunRow {
                id: r.get(0)?,
                graph_run_id: r.get(1)?,
                node_key: r.get(2)?,
                node_type: r.get(3)?,
                status: NodeRunStatus::parse_str(&r.get::<_, String>(4)?)
                    .unwrap_or(NodeRunStatus::Pending),
                attempt: r.get(5)?,
                participant_id: r.get(6)?,
                workspace_id: r.get(7)?,
                lease_token: r.get(8)?,
                lease_expires_at: r.get(9)?,
                started_at: r.get(10)?,
                heartbeat_at: r.get(11)?,
                completed_at: r.get(12)?,
                failure_type: r.get(13)?,
                failure_detail: r.get(14)?,
                version: r.get(15)?,
            })
        },
    )
    .optional()
    .map_err(super::Error::Sqlite)
}

/// 创建 NodeRun（幂等键 UNIQUE 防重复）。返回 (node_run_id, 是否新建)。
pub fn create_node_run(
    conn: &Connection,
    graph_run_id: &str,
    node_key: &str,
    node_type: NodeType,
    attempt: u32,
    participant_id: Option<&str>,
    workspace_id: Option<&str>,
) -> Result<(String, bool), super::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let inserted = conn
        .execute(
            "INSERT INTO node_runs (id, graph_run_id, node_key, node_type, attempt, \
             participant_id, workspace_id) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                id,
                graph_run_id,
                node_key,
                node_type.as_str(),
                attempt,
                participant_id,
                workspace_id
            ],
        )
        .map_err(super::Error::Sqlite)?;
    Ok((id, inserted > 0))
}

/// 列出图的所有 NodeRun（按 attempt、创建顺序）。
pub fn list_node_runs(
    conn: &Connection,
    graph_run_id: &str,
) -> Result<Vec<NodeRunRow>, super::Error> {
    let mut stmt = conn
        .prepare(
            "SELECT id, graph_run_id, node_key, node_type, status, attempt, participant_id, \
             workspace_id, lease_token, lease_expires_at, started_at, heartbeat_at, completed_at, \
             failure_type, failure_detail, version \
             FROM node_runs WHERE graph_run_id=?1 ORDER BY rowid",
        )
        .map_err(super::Error::Sqlite)?;
    let rows = stmt
        .query_map(params![graph_run_id], |r| {
            Ok(NodeRunRow {
                id: r.get(0)?,
                graph_run_id: r.get(1)?,
                node_key: r.get(2)?,
                node_type: r.get(3)?,
                status: NodeRunStatus::parse_str(&r.get::<_, String>(4)?)
                    .unwrap_or(NodeRunStatus::Pending),
                attempt: r.get(5)?,
                participant_id: r.get(6)?,
                workspace_id: r.get(7)?,
                lease_token: r.get(8)?,
                lease_expires_at: r.get(9)?,
                started_at: r.get(10)?,
                heartbeat_at: r.get(11)?,
                completed_at: r.get(12)?,
                failure_type: r.get(13)?,
                failure_detail: r.get(14)?,
                version: r.get(15)?,
            })
        })
        .map_err(super::Error::Sqlite)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(super::Error::Sqlite)?;
    Ok(rows)
}

/// GraphRun 收敛规则（design_graph.md §5.4）：每轮调度末尾计算。
/// 返回收敛后的状态；None = 仍在运行（不需要变更）。
pub fn converge_graph_run(
    conn: &Connection,
    graph_run_id: &str,
) -> Result<Option<GraphRunStatus>, super::Error> {
    let current: String = conn
        .query_row(
            "SELECT status FROM graph_runs WHERE id=?1",
            params![graph_run_id],
            |r| r.get(0),
        )
        .map_err(super::Error::Sqlite)?;
    let current_status = GraphRunStatus::parse_str(&current).unwrap_or(GraphRunStatus::Draft);
    if current_status.is_terminal() {
        return Ok(None);
    }

    let runs = list_node_runs(conn, graph_run_id)?;
    if runs.is_empty() {
        return Ok(None);
    }
    // pending/ready = 尚未执行（等调度）；active = 真正在执行/验证。
    // 只要还有未终态节点，图就不应收敛为 completed。
    let has_unfinished = runs.iter().any(|r| !r.status.is_terminal());
    let has_active = runs
        .iter()
        .any(|r| matches!(r.status, NodeRunStatus::Running | NodeRunStatus::Verifying));
    let has_waiting = runs
        .iter()
        .any(|r| r.status == NodeRunStatus::WaitingApproval);
    let has_failure = runs.iter().any(|r| {
        matches!(
            r.status,
            NodeRunStatus::Failed | NodeRunStatus::Blocked | NodeRunStatus::TimedOut
        )
    });

    let next = if has_unfinished {
        // 有未终态节点：运行中（或等审批 → paused）
        if has_waiting && !has_active {
            Some(GraphRunStatus::Paused)
        } else {
            None
        }
    } else if has_failure {
        Some(GraphRunStatus::Failed)
    } else {
        // 全部 terminal 且无失败 → completed
        Some(GraphRunStatus::Completed)
    };

    if let Some(status) = next {
        conn.execute(
            "UPDATE graph_runs SET status=?1, completed_at=CASE WHEN ?2 THEN ?3 ELSE completed_at END \
             WHERE id=?4",
            params![
                status.as_str(),
                status.is_terminal(),
                unix_now(),
                graph_run_id
            ],
        )
        .map_err(super::Error::Sqlite)?;
        super::events::append(
            conn,
            graph_run_id,
            &format!("graph_{}", status.as_str()),
            None,
            &serde_json::json!({}),
        )?;
        Ok(Some(status))
    } else {
        Ok(None)
    }
}

fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}
