//! `/api/v1/msg/*` handler。

use crate::identity::mailbox as mailbox_db;
use crate::notify;
use crate::proto::{AskOptions, InboxFilter, ServerMsg};
use crate::routing::{inbox, lookup, reply, send, SendRequest};
use crate::server::handlers::{authenticate_req, not_supported};
use crate::server::state::AppState;
use crate::transport::wake::SseEvent;
use axum::http::HeaderMap;

#[allow(clippy::too_many_arguments)]
pub fn handle_send(
    state: &AppState,
    headers: &HeaderMap,
    to: String,
    body: String,
    _subject: Option<String>,
    _files: Vec<String>,
    notify: Option<bool>,
    send_enter: Option<bool>,
    more: bool,
) -> ServerMsg {
    let session = match authenticate_req(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let to_name = match mailbox_db::get_by_address(&state.storage, &to) {
        Ok(Some(mb)) => mb.name,
        Ok(None) => {
            return ServerMsg::Error {
                code: "send_failed".into(),
                message: format!("目标 mailbox 不存在: {}", to),
            }
        }
        Err(e) => {
            return ServerMsg::Error {
                code: "send_failed".into(),
                message: e.to_string(),
            }
        }
    };

    let req = SendRequest {
        to: &to,
        to_name: &to_name,
        from: &session.address,
        from_name: &session.name,
        body: &body,
        content_type: "text",
        reply_to_id: None,
        metadata: "{}",
        more_coming: more,
    };

    match send::send(&state.storage, req) {
        Ok(msg) => {
            state.registry.notify(
                &to,
                SseEvent {
                    message: msg.clone(),
                },
            );
            if notify.unwrap_or(true) {
                let storage = state.storage.clone();
                let to = to.clone();
                let from_name = session.name.clone();
                let message_id = msg.id.clone();
                let limiter = state.notify_limiter.clone();
                tokio::spawn(async move {
                    if let Err(e) = notify::trigger(
                        &storage,
                        &to,
                        &from_name,
                        &message_id,
                        &limiter,
                        send_enter,
                    )
                    .await
                    {
                        tracing::debug!("notify trigger skipped: {}", e);
                    }
                });
            }
            ServerMsg::Ok { id: msg.id }
        }
        Err(e) => ServerMsg::Error {
            code: "send_failed".into(),
            message: e.to_string(),
        },
    }
}

pub fn handle_reply(
    state: &AppState,
    headers: &HeaderMap,
    message_id: String,
    body: String,
    _files: Vec<String>,
    notify: Option<bool>,
    send_enter: Option<bool>,
) -> ServerMsg {
    let session = match authenticate_req(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let resolved = match lookup::resolve_id(&state.storage, &session.address, &message_id) {
        Ok(full_id) => full_id,
        Err(e) => {
            return ServerMsg::Error {
                code: routing_error_code(&e).into(),
                message: e.to_string(),
            }
        }
    };
    match reply::reply(
        &state.storage,
        &resolved,
        &session.address,
        &session.name,
        &body,
        None,
    ) {
        Ok(msg) => {
            state.registry.notify(
                &msg.to_address,
                SseEvent {
                    message: msg.clone(),
                },
            );
            if notify.unwrap_or(true) {
                let storage = state.storage.clone();
                let to = msg.to_address.clone();
                let from_name = session.name.clone();
                let message_id = msg.id.clone();
                let limiter = state.notify_limiter.clone();
                tokio::spawn(async move {
                    if let Err(e) = notify::trigger(
                        &storage,
                        &to,
                        &from_name,
                        &message_id,
                        &limiter,
                        send_enter,
                    )
                    .await
                    {
                        tracing::debug!("notify trigger skipped: {}", e);
                    }
                });
            }
            ServerMsg::Ok { id: msg.id }
        }
        Err(e) => ServerMsg::Error {
            code: "reply_failed".into(),
            message: e.to_string(),
        },
    }
}

pub fn handle_done(
    state: &AppState,
    headers: &HeaderMap,
    message_id: Option<String>,
    _body: Option<String>,
    _files: Vec<String>,
) -> ServerMsg {
    let session = match authenticate_req(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let target_id = match message_id {
        Some(id) => match lookup::resolve_id(&state.storage, &session.address, &id) {
            Ok(full_id) => full_id,
            Err(e) => {
                return ServerMsg::Error {
                    code: routing_error_code(&e).into(),
                    message: e.to_string(),
                }
            }
        },
        None => {
            // 未指定 message_id 时，取最新一条未完成的
            match lookup::detail_and_mark_read(&state.storage, &session.address, "-") {
                Ok(Some(m)) => m.id,
                Ok(None) => {
                    return ServerMsg::Error {
                        code: "inbox_empty".into(),
                        message: "当前 inbox 没有可标记完成的消息".into(),
                    }
                }
                Err(e) => {
                    return ServerMsg::Error {
                        code: "done_failed".into(),
                        message: e.to_string(),
                    }
                }
            }
        }
    };

    match inbox::mark_done(&state.storage, &target_id, &session.address) {
        Ok(()) => ServerMsg::Ok { id: target_id },
        Err(e) => ServerMsg::Error {
            code: "done_failed".into(),
            message: e.to_string(),
        },
    }
}

#[allow(clippy::too_many_arguments)]
pub fn handle_ask(
    state: &AppState,
    headers: &HeaderMap,
    message: String,
    _questions: Vec<String>,
    options: AskOptions,
    _wait: bool,
    _timeout: Option<u64>,
    notify: bool,
) -> ServerMsg {
    let session = match authenticate_req(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };

    let human_address = match mailbox_db::ensure_human(&state.storage, &state.config.human) {
        Ok(addr) => addr,
        Err(e) => {
            return ServerMsg::Error {
                code: "ask_failed".into(),
                message: e.to_string(),
            }
        }
    };

    let content_type = if options.options.is_empty() {
        "text"
    } else {
        "approval_request"
    };
    let metadata = serde_json::json!({
        "choices": options.options,
        "recommended": options.recommended,
        "single": options.single,
        "select_only": options.select_only,
    })
    .to_string();

    let req = SendRequest {
        to: &human_address,
        to_name: &state.config.human.name,
        from: &session.address,
        from_name: &session.name,
        body: &message,
        content_type,
        reply_to_id: None,
        metadata: &metadata,
        more_coming: false,
    };
    match send::send(&state.storage, req) {
        Ok(msg) => {
            state.registry.notify(
                &human_address,
                SseEvent {
                    message: msg.clone(),
                },
            );
            if notify {
                let storage = state.storage.clone();
                let from_name = session.name.clone();
                let message_id = msg.id.clone();
                let limiter = state.notify_limiter.clone();
                tokio::spawn(async move {
                    if let Err(e) = notify::trigger(
                        &storage,
                        &human_address,
                        &from_name,
                        &message_id,
                        &limiter,
                        None,
                    )
                    .await
                    {
                        tracing::debug!("notify trigger skipped: {}", e);
                    }
                });
            }
            ServerMsg::AskResult { message_id: msg.id }
        }
        Err(e) => ServerMsg::Error {
            code: "ask_failed".into(),
            message: e.to_string(),
        },
    }
}

pub fn handle_inbox(state: &AppState, headers: &HeaderMap, filter: InboxFilter) -> ServerMsg {
    let session = match authenticate_req(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match inbox::inbox(&state.storage, &session.address, filter.all) {
        Ok(msgs) => ServerMsg::InboxResult { messages: msgs },
        Err(e) => ServerMsg::Error {
            code: "inbox_failed".into(),
            message: e.to_string(),
        },
    }
}

pub fn handle_read(state: &AppState, headers: &HeaderMap, message_id: Option<String>) -> ServerMsg {
    let session = match authenticate_req(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };

    if let Some(id) = message_id {
        let resolved = match lookup::resolve_id(&state.storage, &session.address, &id) {
            Ok(full_id) => full_id,
            Err(e) => {
                return ServerMsg::Error {
                    code: routing_error_code(&e).into(),
                    message: e.to_string(),
                }
            }
        };
        match lookup::detail_and_mark_read(&state.storage, &session.address, &resolved) {
            Ok(Some(msg)) => ServerMsg::MsgDetail(msg),
            Ok(None) => ServerMsg::Error {
                code: "message_not_found".into(),
                message: "消息不存在".into(),
            },
            Err(e) => ServerMsg::Error {
                code: "read_failed".into(),
                message: e.to_string(),
            },
        }
    } else {
        // 读取所有未读并标记 read；空 inbox 返回稳定错误码 inbox_empty
        match inbox::inbox(&state.storage, &session.address, false) {
            Ok(msgs) if msgs.is_empty() => ServerMsg::Error {
                code: "inbox_empty".into(),
                message: "当前 inbox 没有可查看的消息".into(),
            },
            Ok(msgs) => {
                let ids: Vec<String> = msgs.iter().map(|m| m.id.clone()).collect();
                for id in &ids {
                    let _ = inbox::mark_read(&state.storage, id);
                }
                ServerMsg::InboxResult { messages: msgs }
            }
            Err(e) => ServerMsg::Error {
                code: "read_failed".into(),
                message: e.to_string(),
            },
        }
    }
}

pub fn handle_wait(
    _state: &AppState,
    _headers: &HeaderMap,
    _message_id: Option<String>,
    _timeout: Option<u64>,
    _since: Option<i64>,
) -> ServerMsg {
    not_supported("服务端阻塞 wait")
}

fn routing_error_code(e: &crate::routing::RoutingError) -> &'static str {
    use crate::routing::RoutingError;
    match e {
        RoutingError::MessageNotFound(_) => "message_not_found",
        RoutingError::MessageIdTooShort(_) => "message_id_too_short",
        RoutingError::MessageIdAmbiguous { .. } => "message_id_ambiguous",
        _ => "msg_failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgConfig;
    use crate::server::handlers::id;
    use crate::storage::Storage;
    use tempfile::TempDir;

    fn test_state() -> (AppState, TempDir) {
        let tmp = TempDir::new().unwrap();
        let dot = tmp.path().join(".agtalk");
        let storage = Storage::open_in_memory().unwrap();
        let state = AppState::new(storage, AgConfig::default(), dot);
        (state, tmp)
    }

    fn join(state: &AppState, name: &str) -> String {
        let pid = std::process::id();
        let start_time = current_pid_start_time();
        match id::handle_join(
            state,
            &state.dot_agtalk,
            Some(name.into()),
            Some("intro".into()),
            "none".into(),
            None,
            pid,
            start_time,
        ) {
            ServerMsg::Identity { address, .. } => address,
            other => panic!("expected Identity, got {:?}", other),
        }
    }

    fn current_pid_start_time() -> u64 {
        use sysinfo::{Pid, System};
        let pid = std::process::id();
        let mut sys = System::new_all();
        sys.refresh_processes();
        sys.process(Pid::from(pid as usize))
            .map(|p| p.start_time())
            .unwrap_or(1)
    }

    fn auth_headers_for(state: &AppState, name: &str) -> HeaderMap {
        let session = crate::identity::session_file::read(&state.dot_agtalk, name).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("X-AgTalk-Address", session.address.parse().unwrap());
        headers
    }

    #[tokio::test]
    async fn send_default_returns_ok() {
        let (state, _tmp) = test_state();
        join(&state, "sender");
        let recv_addr = join(&state, "recv");

        let headers = auth_headers_for(&state, "sender");
        let msg = handle_send(
            &state,
            &headers,
            recv_addr,
            "hi".into(),
            None,
            vec![],
            None,
            None,
            false,
        );
        match msg {
            ServerMsg::Ok { id } => assert!(!id.is_empty()),
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    #[test]
    fn send_with_notify_false_returns_ok() {
        let (state, _tmp) = test_state();
        join(&state, "sender");
        let recv_addr = join(&state, "recv");

        let headers = auth_headers_for(&state, "sender");
        let msg = handle_send(
            &state,
            &headers,
            recv_addr,
            "hi".into(),
            None,
            vec![],
            Some(false),
            None,
            false,
        );
        match msg {
            ServerMsg::Ok { id } => assert!(!id.is_empty()),
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    #[test]
    fn read_with_short_id() {
        let (state, _tmp) = test_state();
        join(&state, "sender");
        let recv_addr = join(&state, "recv");

        let sender_headers = auth_headers_for(&state, "sender");
        let send_resp = handle_send(
            &state,
            &sender_headers,
            recv_addr.clone(),
            "hello short id".into(),
            None,
            vec![],
            Some(false),
            None,
            false,
        );
        let sent_id = match send_resp {
            ServerMsg::Ok { id } => id,
            other => panic!("expected Ok, got {:?}", other),
        };
        let short_id = &sent_id[..8];

        let recv_headers = auth_headers_for(&state, "recv");
        let read_resp = handle_read(&state, &recv_headers, Some(short_id.to_string()));
        match read_resp {
            ServerMsg::MsgDetail(msg) => assert_eq!(msg.id, sent_id),
            other => panic!("expected MsgDetail, got {:?}", other),
        }
    }

    #[test]
    fn reply_with_short_id() {
        let (state, _tmp) = test_state();
        join(&state, "sender");
        let recv_addr = join(&state, "recv");

        let sender_headers = auth_headers_for(&state, "sender");
        handle_send(
            &state,
            &sender_headers,
            recv_addr.clone(),
            "hello".into(),
            None,
            vec![],
            Some(false),
            None,
            false,
        );

        let recv_headers = auth_headers_for(&state, "recv");
        let inbox_resp = handle_inbox(&state, &recv_headers, Default::default());
        let first_id = match inbox_resp {
            ServerMsg::InboxResult { messages } => messages[0].id.clone(),
            other => panic!("expected InboxResult, got {:?}", other),
        };
        let short_id = &first_id[..8];

        let reply_resp = handle_reply(
            &state,
            &recv_headers,
            short_id.to_string(),
            "ok".into(),
            vec![],
            Some(false),
            None,
        );
        match reply_resp {
            ServerMsg::Ok { id } => assert!(!id.is_empty()),
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    #[test]
    fn done_with_short_id() {
        let (state, _tmp) = test_state();
        join(&state, "sender");
        let recv_addr = join(&state, "recv");

        let sender_headers = auth_headers_for(&state, "sender");
        handle_send(
            &state,
            &sender_headers,
            recv_addr.clone(),
            "hello".into(),
            None,
            vec![],
            Some(false),
            None,
            false,
        );

        let recv_headers = auth_headers_for(&state, "recv");
        let inbox_resp = handle_inbox(&state, &recv_headers, Default::default());
        let first_id = match inbox_resp {
            ServerMsg::InboxResult { messages } => messages[0].id.clone(),
            other => panic!("expected InboxResult, got {:?}", other),
        };
        let short_id = &first_id[..8];

        let done_resp = handle_done(
            &state,
            &recv_headers,
            Some(short_id.to_string()),
            None,
            vec![],
        );
        match done_resp {
            ServerMsg::Ok { id } => assert_eq!(id, first_id),
            other => panic!("expected Ok, got {:?}", other),
        }
    }

    #[test]
    fn read_short_id_ambiguous_returns_error() {
        let (state, _tmp) = test_state();
        join(&state, "sender");
        let recv_addr = join(&state, "recv");

        let sender_addr = crate::identity::session_file::read(&state.dot_agtalk, "sender")
            .unwrap()
            .address;
        // 直接插入两条前缀相同的消息（UUID v4 前 8 位不可控）。
        for (i, id) in [
            "6f0d4353-1111-46ab-afb6-8c7f6af02049",
            "6f0d4353-2222-46ab-afb6-8c7f6af02049",
        ]
        .iter()
        .enumerate()
        {
            state
                .storage
                .conn()
                .execute(
                    "INSERT INTO messages (id, to_address, to_name, from_address, from_name, body, content_type, reply_to_id, metadata, event_id, status, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'pending', ?11)",
                    rusqlite::params![id, &recv_addr, "recv", &sender_addr, "sender", "hi", "text", Option::<String>::None, "{}", (i + 1) as i64, 1.0],
                )
                .unwrap();
        }

        let recv_headers = auth_headers_for(&state, "recv");
        let read_resp = handle_read(&state, &recv_headers, Some("6f0d4353".into()));
        match read_resp {
            ServerMsg::Error { code, .. } => assert_eq!(code, "message_id_ambiguous"),
            other => panic!("expected Error, got {:?}", other),
        }
    }
}
