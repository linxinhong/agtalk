//! human 发信与 agent 列表：主动发信（跨端事件幂等）、在线 agent 查询。

use super::{after_human_reply, authenticate_human, err, human_error_msg};
use crate::human;
use crate::identity::mailbox as mailbox_db;
use crate::proto::ServerMsg;
use crate::routing::{lookup, send, SendRequest};
use crate::server::handlers::id as id_handlers;
use crate::server::state::AppState;
use axum::http::HeaderMap;

/// 在线 agent 列表：活跃 mailbox（left_at 为空），排除 human 自身。
pub fn handle_agents(state: &AppState, headers: &HeaderMap) -> ServerMsg {
    let session = match authenticate_human(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match lookup::lookup(&state.storage, None) {
        Ok(mbs) => {
            let mailboxes = mbs
                .iter()
                .filter(|mb| mb.left_at.is_none() && mb.address != session.address)
                .map(id_handlers::build_lookup_mailbox)
                .collect();
            ServerMsg::LookupResult { mailboxes }
        }
        Err(e) => err("lookup_failed", e),
    }
}

/// human 主动发信：目标必须是活跃 mailbox，复用 routing（UUID 路由）。
/// 提供 surface+external_event_id 时走同事务幂等路径：重复事件回放原消息 id，不重复发信。
pub fn handle_send(
    state: &AppState,
    headers: &HeaderMap,
    to: String,
    body: String,
    subject: Option<String>,
    surface: Option<String>,
    external_event_id: Option<String>,
) -> ServerMsg {
    let session = match authenticate_human(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };
    if to == session.address {
        return ServerMsg::Error {
            code: "agent_not_found".into(),
            message: "目标必须是活跃 agent，不能向 human 自身发信".into(),
        };
    }
    let target = match mailbox_db::get_by_address(&state.storage, &to) {
        Ok(Some(mb)) if mb.left_at.is_none() => mb,
        Ok(_) => {
            return ServerMsg::Error {
                code: "agent_not_found".into(),
                message: format!("目标 agent 不存在或已离开: {}", to),
            };
        }
        Err(e) => return err("send_failed", e),
    };
    let req = SendRequest {
        to: &to,
        to_name: &target.name,
        from: &session.address,
        from_name: &session.name,
        body: &body,
        content_type: "text",
        reply_to_id: None,
        subject: subject.as_deref(),
        metadata: "{}",
        more_coming: false,
    };
    let result: Result<(crate::routing::Message, bool), ServerMsg> =
        match (&surface, &external_event_id) {
            (Some(surface), Some(eid)) => {
                human::send_with_receipt(&state.storage, &req, surface, eid)
                    .map_err(|e| human_error_msg(&e))
            }
            _ => send::send(&state.storage, req)
                .map(|m| (m, false))
                .map_err(|e| err("send_failed", e)),
        };
    match result {
        Ok((msg, deduplicated)) => {
            // 重复事件回放：消息未重复创建，也不再触发 SSE/notify
            if !deduplicated {
                after_human_reply(state, &msg);
            }
            ServerMsg::Ok { id: msg.id }
        }
        Err(e) => e,
    }
}

#[cfg(test)]
mod tests {
    use crate::identity::mailbox::{create, mark_left};
    use crate::proto::ServerMsg;
    use crate::routing::inbox;
    use crate::server::handlers::human::testkit::*;

    #[tokio::test]
    async fn human_send_reaches_agent_inbox() {
        let fx = setup();
        let agent_addr = create(&fx.state.storage, "nora", "", "").unwrap();

        match super::handle_send(
            &fx.state,
            &headers_with(&fx.token),
            agent_addr.clone(),
            "hello nora".into(),
            Some("greet".into()),
            None,
            None,
        ) {
            ServerMsg::Ok { id } => {
                let inbox = inbox::inbox(&fx.state.storage, &agent_addr, false).unwrap();
                assert_eq!(inbox.len(), 1);
                assert_eq!(inbox[0].id, id);
                assert_eq!(inbox[0].from_address, fx.human_addr);
                assert_eq!(inbox[0].subject.as_deref(), Some("greet"));
            }
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    #[test]
    fn human_send_rejects_left_or_unknown_agent() {
        let fx = setup();
        let left_addr = create(&fx.state.storage, "gone", "", "").unwrap();
        mark_left(&fx.state.storage, &left_addr).unwrap();

        for to in [left_addr, "no-such-agent".to_string()] {
            match super::handle_send(
                &fx.state,
                &headers_with(&fx.token),
                to,
                "hi".into(),
                None,
                None,
                None,
            ) {
                ServerMsg::Error { code, .. } => assert_eq!(code, "agent_not_found"),
                other => panic!("expected agent_not_found, got {:?}", other),
            }
        }
    }

    #[test]
    fn human_send_rejects_human_itself() {
        let fx = setup();
        match super::handle_send(
            &fx.state,
            &headers_with(&fx.token),
            fx.human_addr.clone(),
            "talk to myself".into(),
            None,
            None,
            None,
        ) {
            ServerMsg::Error { code, message } => {
                assert_eq!(code, "agent_not_found");
                assert!(message.contains("human 自身"));
            }
            other => panic!("expected agent_not_found, got {:?}", other),
        }
    }

    #[test]
    fn agents_list_excludes_human_and_left() {
        let fx = setup();
        create(&fx.state.storage, "active", "", "").unwrap();
        let left_addr = create(&fx.state.storage, "left", "", "").unwrap();
        mark_left(&fx.state.storage, &left_addr).unwrap();

        match super::handle_agents(&fx.state, &headers_with(&fx.token)) {
            ServerMsg::LookupResult { mailboxes } => {
                assert_eq!(mailboxes.len(), 1);
                assert_eq!(mailboxes[0].name, "active");
            }
            other => panic!("expected LookupResult, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn human_send_duplicate_event_replays_original_message_id() {
        let fx = setup();
        let agent_addr = create(&fx.state.storage, "nora", "", "").unwrap();

        let first = super::handle_send(
            &fx.state,
            &headers_with(&fx.token),
            agent_addr.clone(),
            "hello".into(),
            None,
            Some("feishu".into()),
            Some("evt-send-1".into()),
        );
        let first_id = match first {
            ServerMsg::Ok { id } => id,
            other => panic!("expected Ok, got {:?}", other),
        };

        // 重复事件：回放原 message id，不重复发信
        let second = super::handle_send(
            &fx.state,
            &headers_with(&fx.token),
            agent_addr.clone(),
            "hello again".into(),
            None,
            Some("feishu".into()),
            Some("evt-send-1".into()),
        );
        match second {
            ServerMsg::Ok { id } => assert_eq!(id, first_id),
            other => panic!("expected Ok, got {:?}", other),
        }
        let inbox = inbox::inbox(&fx.state.storage, &agent_addr, false).unwrap();
        assert_eq!(inbox.len(), 1, "重复事件不应产生第二条消息");
    }
}
