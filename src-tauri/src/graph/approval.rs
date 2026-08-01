//! Approval Node：复用 human approval 仲裁（docs/design_graph.md §5.2 取舍 3）。
//!
//! 派发：approval 节点不找 participant，向 human mailbox 发 `approval_request`
//! （复用现有 msg/ask 投递链：popup/feishu fanout），节点 → waiting_approval，
//! assignment(kind=approval) 关联请求消息。
//! 回执：human reply/cancel 后（actions.rs 挂钩）查 approval_resolutions →
//! 通过 → waiting_approval → running → succeeded；拒绝/取消 → failed(approval_rejected)。
//! 不建第二套审批。

use crate::graph::compiler::CompiledGraph;
use crate::graph::events;
use crate::graph::scheduler::{tick, DispatchItem};
use crate::graph::state::{converge_graph_run, get_node_run, transition, NodeRunStatus};
use crate::identity::auth::AuthenticatedSession;
use crate::proto::ServerMsg;
use crate::routing::send::send;
use crate::routing::SendRequest;
use crate::server::state::AppState;
use rusqlite::{params, OptionalExtension};

/// 派发审批节点：发 approval_request 给 human mailbox，节点 → waiting_approval。
#[allow(clippy::result_large_err)]
pub fn dispatch_approval(
    state: &AppState,
    from: &AuthenticatedSession,
    compiled: &CompiledGraph,
    item: &DispatchItem,
) -> Result<(), ServerMsg> {
    let spec_node = compiled
        .nodes
        .iter()
        .find(|n| n.id == item.node_key)
        .ok_or_else(|| {
            err(
                "graph_node_not_found",
                format!("spec 无节点 {}", item.node_key),
            )
        })?;
    let approval = spec_node.approval.as_ref().ok_or_else(|| {
        err(
            "approval_message_required",
            "approval 节点缺少 approval.message",
        )
    })?;
    let human_addr = crate::human::human_address(&state.storage)
        .map_err(|e| err("graph_human_unavailable", e.to_string()))?;

    // 无锁：send（内部自取锁）
    let metadata = serde_json::json!({ "options": approval.options }).to_string();
    let req = SendRequest {
        to: &human_addr,
        to_name: "human",
        from: &from.address,
        from_name: &from.name,
        body: &approval.message,
        content_type: "approval_request",
        reply_to_id: None,
        subject: Some("[graph 审批]"),
        metadata: &metadata,
        more_coming: false,
    };
    let msg = send(&state.storage, req).map_err(|e| err("graph_dispatch_send", e.to_string()))?;
    // 审批提醒（Tim 评审偏差②：人类审批也应有 notify 打扰）
    {
        // 审批提醒（同 dispatch：有 runtime 才触发）
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let limiter = state.notify_limiter.clone();
            let storage = state.storage.clone();
            let to = human_addr.clone();
            let from_name = from.name.clone();
            let message_id = msg.id.clone();
            handle.spawn(async move {
                let _ = crate::notify::trigger(
                    &storage,
                    &to,
                    &from_name,
                    &message_id,
                    &limiter,
                    Some(true),
                )
                .await;
            });
        }
    }

    // 锁内：节点 → waiting_approval + assignment
    let conn = state.storage.conn();
    let run = get_node_run(&conn, &item.node_run_id)
        .map_err(graph_err)?
        .ok_or_else(|| err("graph_node_not_found", "节点不存在"))?;
    let _ = transition(
        &conn,
        &run.id,
        run.version,
        NodeRunStatus::Dispatched,
        NodeRunStatus::WaitingApproval,
        None,
        None,
        None,
    )
    .map_err(graph_err)?;
    conn.execute(
        "INSERT INTO graph_node_assignments (id, node_run_id, message_id, kind) \
         VALUES (?1,?2,?3,'approval')",
        params![uuid::Uuid::new_v4().to_string(), item.node_run_id, msg.id],
    )
    .map_err(sqlite_err)?;
    events::append(
        &conn,
        &item.graph_run_id,
        "node_waiting_approval",
        Some(&item.node_key),
        &serde_json::json!({ "message_id": msg.id }),
    )
    .map_err(graph_err)?;
    let _ = converge_graph_run(&conn, &item.graph_run_id).map_err(graph_err)?;
    Ok(())
}

/// human 回复/取消回调（actions.rs 挂钩）：approval 消息解决后推进图节点。
/// 返回是否处理了图节点（非图消息返回 false）。
pub fn notify_human_reply(state: &AppState, request_message_id: &str) -> Result<bool, String> {
    // 1. 查 assignment（kind=approval）
    let node_run_id: Option<String> = {
        let conn = state.storage.conn();
        conn.query_row(
            "SELECT node_run_id FROM graph_node_assignments \
             WHERE message_id=?1 AND kind='approval'",
            params![request_message_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
    };
    let Some(node_run_id) = node_run_id else {
        return Ok(false); // 非图审批消息
    };

    // 2. 读取节点（幂等：非 waiting_approval 不处理）
    let run = {
        let conn = state.storage.conn();
        get_node_run(&conn, &node_run_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "图节点不存在".to_string())?
    };
    if run.status != NodeRunStatus::WaitingApproval {
        return Ok(true);
    }

    // 3. 查 resolution：cancelled 视为拒绝
    let resolution: String = {
        let conn = state.storage.conn();
        conn.query_row(
            "SELECT resolution FROM approval_resolutions WHERE request_message_id=?1",
            params![request_message_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?
    };
    let approved = resolution != "cancelled" && !resolution.trim().is_empty();

    // 4. 推进（锁内状态迁移）
    let graph_run_id = run.graph_run_id.clone();
    {
        let conn = state.storage.conn();
        if approved {
            // approval 节点无执行体：审批通过即完成（状态机 WaitingApproval→Succeeded）
            let _ = transition(
                &conn,
                &run.id,
                run.version,
                NodeRunStatus::WaitingApproval,
                NodeRunStatus::Succeeded,
                None,
                None,
                None,
            )
            .map_err(|e| e.to_string())?;
        } else {
            let _ = transition(
                &conn,
                &run.id,
                run.version,
                NodeRunStatus::WaitingApproval,
                NodeRunStatus::Failed,
                Some("approval_rejected"),
                Some("审批被取消或拒绝"),
                None,
            )
            .map_err(|e| e.to_string())?;
        }
        let _ = converge_graph_run(&conn, &graph_run_id).map_err(|e| e.to_string())?;
    }

    // 5. 无锁态：tick + 派发下游（from = system/human）
    let cg = load_compiled_graph(state, &graph_run_id)?;
    let items = {
        let conn = state.storage.conn();
        tick(&conn, &graph_run_id, &cg).map_err(|e| e.to_string())?
    };
    if !items.is_empty() {
        let from = system_from(state)?;
        for item in &items {
            crate::server::handlers::graph_dispatch::dispatch_one(state, &from, &cg, item)
                .map_err(|e| format!("{:?}", e))?;
        }
    }
    let since = crate::server::handlers::graph::graph_max_event_id(state, &graph_run_id);
    crate::server::handlers::graph::push_new_graph_events(state, &graph_run_id, since);
    Ok(true)
}

fn load_compiled_graph(state: &AppState, run_id: &str) -> Result<CompiledGraph, String> {
    let conn = state.storage.conn();
    let json: String = conn
        .query_row(
            "SELECT compiled_graph FROM graph_runs WHERE id=?1",
            params![run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&json).map_err(|e| e.to_string())
}

/// 系统身份：human mailbox（daemon 代表系统派发下游）。
fn system_from(state: &AppState) -> Result<AuthenticatedSession, String> {
    let address = crate::human::human_address(&state.storage)
        .map_err(|e| format!("human mailbox 不可用: {e}"))?;
    Ok(AuthenticatedSession {
        address,
        name: "system".into(),
        workspace: String::new(),
        workspace_root: std::path::PathBuf::new(),
        pid: None,
    })
}

fn err(code: &str, message: impl Into<String>) -> ServerMsg {
    ServerMsg::Error {
        code: code.to_string(),
        message: message.into(),
    }
}

fn sqlite_err(e: rusqlite::Error) -> ServerMsg {
    err("graph_db_error", e.to_string())
}

fn graph_err(e: crate::graph::Error) -> ServerMsg {
    match e {
        crate::graph::Error::Sqlite(e) => err("graph_db_error", e.to_string()),
        crate::graph::Error::Json(e) => err("graph_json_error", e.to_string()),
        other => err("graph_error", other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::auth::AuthenticatedSession;
    use crate::server::state::AppState;
    use crate::storage::Storage;

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
            conn.execute(
                "INSERT INTO mailboxes (address, name) VALUES ('human-addr', 'human')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO event_sequences (address) VALUES ('human-addr')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO system_mailboxes (role, address) VALUES ('human', 'human-addr')",
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

    fn approval_spec() -> String {
        r#"
version: 1
goal: "审批闭环"
nodes:
  - id: gate
    type: approval
    approval: { message: "允许发布？", options: [approve, reject] }
    timeout_seconds: 3600
  - id: release
    type: executor
    dependencies: [gate]
    outputs: { schema: s }
    executor_requirements: { participant: agent-x }
    workspace: w
    write_paths: [x]
    acceptance: [{ type: path }]
    timeout_seconds: 300
"#
        .to_string()
    }

    #[test]
    fn approval_node_dispatches_to_human_and_waits() {
        let state = test_state();
        // 提交含 approval 节点的图
        let run_id = match crate::server::handlers::graph::submit_and_start(
            &state,
            &fake_session(),
            &approval_spec(),
        )
        .unwrap()
        {
            ServerMsg::GraphRunCreated { run_id, .. } => run_id,
            other => panic!("预期 GraphRunCreated: {other:?}"),
        };

        let conn = state.storage.conn();
        // gate → waiting_approval
        let gate = crate::graph::state::get_node_run_by_key(&conn, &run_id, "gate", 1)
            .unwrap()
            .unwrap();
        assert_eq!(gate.status, NodeRunStatus::WaitingApproval);
        // human inbox 有 approval_request
        let cnt: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE to_address='human-addr' AND content_type='approval_request'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cnt, 1, "应有一条审批请求");
        // assignment kind=approval
        let kind: String = conn
            .query_row(
                "SELECT kind FROM graph_node_assignments WHERE node_run_id=?1",
                params![gate.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kind, "approval");
        // release 未派发（等审批）
        let release = crate::graph::state::get_node_run_by_key(&conn, &run_id, "release", 1)
            .unwrap()
            .unwrap();
        assert_eq!(release.status, NodeRunStatus::Pending);
        // 图 paused（等审批）
        let gstatus: String = conn
            .query_row(
                "SELECT status FROM graph_runs WHERE id=?1",
                params![run_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(gstatus, "paused");
    }

    #[test]
    fn approval_approve_advances_node_and_dispatches_downstream() {
        let state = test_state();
        // 预置 agent-x（下游 release 的 participant）
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
        let run_id = match crate::server::handlers::graph::submit_and_start(
            &state,
            &fake_session(),
            &approval_spec(),
        )
        .unwrap()
        {
            ServerMsg::GraphRunCreated { run_id, .. } => run_id,
            _ => panic!(),
        };
        // 模拟 human 审批通过：直接写 approval_resolutions（reply 流程简化）
        let msg_id: String = {
            let conn = state.storage.conn();
            conn.query_row(
                "SELECT message_id FROM graph_node_assignments WHERE kind='approval' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        {
            let conn = state.storage.conn();
            conn.execute(
                "INSERT INTO approval_resolutions (request_message_id, resolved_by, resolution, selected_choice, response_message_id) \
                 VALUES (?1, 'popup', 'text', 'approve', ?1)",
                params![msg_id],
            )
            .unwrap();
        }
        let handled = notify_human_reply(&state, &msg_id).unwrap();
        assert!(handled, "图审批消息应被处理");

        let conn = state.storage.conn();
        let gate = crate::graph::state::get_node_run_by_key(&conn, &run_id, "gate", 1)
            .unwrap()
            .unwrap();
        assert_eq!(
            gate.status,
            NodeRunStatus::Succeeded,
            "审批通过 → succeeded"
        );
        let release = crate::graph::state::get_node_run_by_key(&conn, &run_id, "release", 1)
            .unwrap()
            .unwrap();
        assert_eq!(
            release.status,
            NodeRunStatus::Dispatched,
            "下游应已派发（release 收到消息）"
        );
    }

    #[test]
    fn approval_cancel_rejects_node() {
        let state = test_state();
        let run_id = match crate::server::handlers::graph::submit_and_start(
            &state,
            &fake_session(),
            &approval_spec(),
        )
        .unwrap()
        {
            ServerMsg::GraphRunCreated { run_id, .. } => run_id,
            _ => panic!(),
        };
        let msg_id: String = {
            let conn = state.storage.conn();
            conn.query_row(
                "SELECT message_id FROM graph_node_assignments WHERE kind='approval' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        {
            let conn = state.storage.conn();
            conn.execute(
                "INSERT INTO approval_resolutions (request_message_id, resolved_by, resolution, selected_choice, response_message_id) \
                 VALUES (?1, 'popup', 'cancelled', NULL, ?1)",
                params![msg_id],
            )
            .unwrap();
        }
        let handled = notify_human_reply(&state, &msg_id).unwrap();
        assert!(handled);

        let conn = state.storage.conn();
        let gate = crate::graph::state::get_node_run_by_key(&conn, &run_id, "gate", 1)
            .unwrap()
            .unwrap();
        assert_eq!(gate.status, NodeRunStatus::Failed);
        assert_eq!(gate.failure_type.as_deref(), Some("approval_rejected"));
        // 下游 release 被 blocked（依赖失败）
        let release = crate::graph::state::get_node_run_by_key(&conn, &run_id, "release", 1)
            .unwrap()
            .unwrap();
        assert_eq!(release.status, NodeRunStatus::Blocked);
    }

    #[test]
    fn non_graph_message_ignored() {
        let state = test_state();
        let handled = notify_human_reply(&state, "not-a-graph-msg").unwrap();
        assert!(!handled);
    }
}
