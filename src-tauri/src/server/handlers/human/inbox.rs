//! human inbox 读取：列表、按 id 读取（标记 read）。

use super::{authenticate_human, err, resolve_human_id};
use crate::proto::ServerMsg;
use crate::routing::{inbox, lookup};
use crate::server::state::AppState;
use axum::http::HeaderMap;

pub fn handle_inbox(state: &AppState, headers: &HeaderMap, all: bool) -> ServerMsg {
    let session = match authenticate_human(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match inbox::inbox(&state.storage, &session.address, all) {
        Ok(msgs) => ServerMsg::InboxResult { messages: msgs },
        Err(e) => err("inbox_failed", e),
    }
}

pub fn handle_read(state: &AppState, headers: &HeaderMap, message_id: Option<String>) -> ServerMsg {
    let session = match authenticate_human(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };
    if let Some(id) = message_id {
        let resolved = match resolve_human_id(state, &session.address, &id) {
            Ok(r) => r,
            Err(e) => return e,
        };
        match lookup::detail_and_mark_read(&state.storage, &session.address, &resolved) {
            Ok(Some((msg, _))) => ServerMsg::MsgDetail(msg),
            Ok(None) => ServerMsg::Error {
                code: "message_not_found".into(),
                message: "消息不存在".into(),
            },
            Err(e) => err("read_failed", e),
        }
    } else {
        match inbox::unread_inbox(&state.storage, &session.address) {
            Ok(msgs) if msgs.is_empty() => ServerMsg::Error {
                code: "inbox_empty".into(),
                message: "当前 inbox 没有可查看的消息".into(),
            },
            Ok(msgs) => {
                for msg in &msgs {
                    let _ = inbox::mark_read(&state.storage, &msg.id);
                }
                ServerMsg::InboxResult { messages: msgs }
            }
            Err(e) => err("read_failed", e),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::identity::mailbox::create;
    use crate::proto::ServerMsg;
    use crate::server::handlers::human::testkit::*;
    use axum::http::HeaderMap;

    #[test]
    fn token_auth_three_states() {
        let fx = setup();
        // 无 token
        match super::handle_inbox(&fx.state, &HeaderMap::new(), false) {
            ServerMsg::Error { code, .. } => assert_eq!(code, "auth_failed"),
            other => panic!("expected auth_failed, got {:?}", other),
        }
        // 错误 token
        match super::handle_inbox(&fx.state, &headers_with("wrong"), false) {
            ServerMsg::Error { code, .. } => assert_eq!(code, "auth_failed"),
            other => panic!("expected auth_failed, got {:?}", other),
        }
        // 正确 token
        match super::handle_inbox(&fx.state, &headers_with(&fx.token), false) {
            ServerMsg::InboxResult { messages } => assert!(messages.is_empty()),
            other => panic!("expected InboxResult, got {:?}", other),
        }
    }

    #[test]
    fn human_read_marks_read_and_empty_inbox_stable() {
        let fx = setup();
        let agent_addr = create(&fx.state.storage, "sender", "", "").unwrap();
        send_to_human(&fx, &agent_addr, "text", "{}");

        match super::handle_read(&fx.state, &headers_with(&fx.token), None) {
            ServerMsg::InboxResult { messages } => assert_eq!(messages.len(), 1),
            other => panic!("expected InboxResult, got {:?}", other),
        }
        match super::handle_read(&fx.state, &headers_with(&fx.token), None) {
            ServerMsg::Error { code, .. } => assert_eq!(code, "inbox_empty"),
            other => panic!("expected inbox_empty, got {:?}", other),
        }
    }
}
