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
            record_send_history(state, &session, &to_name, &msg, "msg.send");
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
    let original = match lookup::detail(&state.storage, &resolved) {
        Ok(Some(m)) => m,
        Ok(None) => {
            return ServerMsg::Error {
                code: "message_not_found".into(),
                message: "消息不存在".into(),
            }
        }
        Err(e) => {
            return ServerMsg::Error {
                code: "reply_failed".into(),
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
            record_reply_history(state, &session, &original, &msg);
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

    let target_msg = match message_id {
        Some(id) => match lookup::resolve_id(&state.storage, &session.address, &id) {
            Ok(full_id) => match lookup::detail(&state.storage, &full_id) {
                Ok(Some(m)) => m,
                Ok(None) => {
                    return ServerMsg::Error {
                        code: "message_not_found".into(),
                        message: "消息不存在".into(),
                    }
                }
                Err(e) => {
                    return ServerMsg::Error {
                        code: "done_failed".into(),
                        message: e.to_string(),
                    }
                }
            },
            Err(e) => {
                return ServerMsg::Error {
                    code: routing_error_code(&e).into(),
                    message: e.to_string(),
                }
            }
        },
        None => {
            // 未指定 message_id 时，取最新一条未完成的（同时标记 read）
            match lookup::detail_and_mark_read(&state.storage, &session.address, "-") {
                Ok(Some(m)) => m,
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

    match inbox::mark_done(&state.storage, &target_msg.id, &session.address) {
        Ok(()) => {
            record_status_history(
                state,
                &session,
                &target_msg,
                &target_msg.status,
                "done",
                "msg.done",
            );
            ServerMsg::Ok { id: target_msg.id }
        }
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
            record_send_history(state, &session, &state.config.human.name, &msg, "msg.ask");
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
            Ok(Some(msg)) => {
                record_status_history(state, &session, &msg, &msg.status, "read", "msg.read");
                ServerMsg::MsgDetail(msg)
            }
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
                for msg in &msgs {
                    let _ = inbox::mark_read(&state.storage, &msg.id);
                    record_status_history(state, &session, msg, &msg.status, "read", "msg.read");
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

fn record_send_history(
    state: &AppState,
    sender: &crate::identity::auth::AuthenticatedSession,
    to_name: &str,
    msg: &crate::routing::Message,
    source: &str,
) {
    let dot = &state.dot_agtalk;
    if let Err(e) = crate::identity::history::append_message(
        dot,
        &sender.name,
        &sender.address,
        "out",
        msg,
        source,
    ) {
        tracing::warn!("sender history append failed: {}", e);
    }
    if let Err(e) =
        crate::identity::history::append_message(dot, to_name, &msg.to_address, "in", msg, source)
    {
        tracing::warn!("receiver history append failed: {}", e);
    }
}

fn record_reply_history(
    state: &AppState,
    sender: &crate::identity::auth::AuthenticatedSession,
    original: &crate::routing::Message,
    reply: &crate::routing::Message,
) {
    let dot = &state.dot_agtalk;
    // reply 发送方的 out 事件
    if let Err(e) = crate::identity::history::append_message(
        dot,
        &sender.name,
        &sender.address,
        "out",
        reply,
        "msg.reply",
    ) {
        tracing::warn!("reply sender history append failed: {}", e);
    }
    // 原消息发送方收到 reply 的 in 事件
    if let Err(e) = crate::identity::history::append_message(
        dot,
        &original.from_name,
        &original.from_address,
        "in",
        reply,
        "msg.reply",
    ) {
        tracing::warn!("reply receiver history append failed: {}", e);
    }
    // 原消息被 reply 后状态变为 read，记录给原消息的接收方（即当前 reply 发送方）
    if let Err(e) = crate::identity::history::append_status(
        dot,
        &sender.name,
        &sender.address,
        &original.id,
        &original.status,
        "read",
        "msg.reply",
    ) {
        tracing::warn!("reply status history append failed: {}", e);
    }
}

fn record_status_history(
    state: &AppState,
    owner: &crate::identity::auth::AuthenticatedSession,
    msg: &crate::routing::Message,
    old_status: &str,
    new_status: &str,
    reason: &str,
) {
    let dot = &state.dot_agtalk;
    if let Err(e) = crate::identity::history::append_status(
        dot,
        &owner.name,
        &owner.address,
        &msg.id,
        old_status,
        new_status,
        reason,
    ) {
        tracing::warn!("status history append failed: {}", e);
    }
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
    fn send_writes_history_for_sender_and_receiver() {
        let (state, _tmp) = test_state();
        join(&state, "sender");
        let recv_addr = join(&state, "recv");

        let headers = auth_headers_for(&state, "sender");
        handle_send(
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

        let sender_history = read_history_lines(&state.dot_agtalk, "sender");
        let receiver_history = read_history_lines(&state.dot_agtalk, "recv");
        assert_eq!(sender_history.len(), 1);
        assert_eq!(receiver_history.len(), 1);
        assert_eq!(sender_history[0]["type"], "message");
        assert_eq!(sender_history[0]["dir"], "out");
        assert_eq!(sender_history[0]["source"], "msg.send");
        assert_eq!(receiver_history[0]["dir"], "in");
        assert_eq!(receiver_history[0]["source"], "msg.send");
    }

    #[test]
    fn reply_writes_history_and_status() {
        let (state, _tmp) = test_state();
        join(&state, "sender");
        let recv_addr = join(&state, "recv");

        let sender_headers = auth_headers_for(&state, "sender");
        let send_resp = handle_send(
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
        let sent_id = match send_resp {
            ServerMsg::Ok { id } => id,
            other => panic!("expected Ok, got {:?}", other),
        };

        let recv_headers = auth_headers_for(&state, "recv");
        handle_read(&state, &recv_headers, Some(sent_id[..8].to_string()));

        let reply_resp = handle_reply(
            &state,
            &recv_headers,
            sent_id[..8].to_string(),
            "ok".into(),
            vec![],
            Some(false),
            None,
        );
        assert!(matches!(reply_resp, ServerMsg::Ok { .. }));

        let sender_history = read_history_lines(&state.dot_agtalk, "sender");
        let recv_history = read_history_lines(&state.dot_agtalk, "recv");

        // sender: original out message, reply in message
        assert!(sender_history
            .iter()
            .any(|h| h["dir"] == "out" && h["source"] == "msg.send"));
        assert!(sender_history
            .iter()
            .any(|h| h["dir"] == "in" && h["source"] == "msg.reply"));

        // receiver: original in message (from send), out reply message, status for original pending->read (from handle_read), status original read->read (from reply)
        assert!(recv_history
            .iter()
            .any(|h| h["dir"] == "in" && h["source"] == "msg.send"));
        assert!(recv_history
            .iter()
            .any(|h| h["dir"] == "out" && h["source"] == "msg.reply"));
        assert!(recv_history
            .iter()
            .any(|h| h["type"] == "status" && h["reason"] == "msg.read"));
        assert!(recv_history
            .iter()
            .any(|h| h["type"] == "status" && h["reason"] == "msg.reply"));
    }

    #[test]
    fn read_writes_status_history() {
        let (state, _tmp) = test_state();
        join(&state, "sender");
        let recv_addr = join(&state, "recv");

        let sender_headers = auth_headers_for(&state, "sender");
        handle_send(
            &state,
            &sender_headers,
            recv_addr,
            "hello".into(),
            None,
            vec![],
            Some(false),
            None,
            false,
        );

        let recv_headers = auth_headers_for(&state, "recv");
        handle_read(&state, &recv_headers, None);

        let recv_history = read_history_lines(&state.dot_agtalk, "recv");
        assert!(recv_history
            .iter()
            .any(|h| h["type"] == "status" && h["to"] == "read" && h["reason"] == "msg.read"));
    }

    #[test]
    fn done_writes_status_history() {
        let (state, _tmp) = test_state();
        join(&state, "sender");
        let recv_addr = join(&state, "recv");

        let sender_headers = auth_headers_for(&state, "sender");
        handle_send(
            &state,
            &sender_headers,
            recv_addr,
            "hello".into(),
            None,
            vec![],
            Some(false),
            None,
            false,
        );

        let recv_headers = auth_headers_for(&state, "recv");
        handle_done(&state, &recv_headers, None, None, vec![]);

        let recv_history = read_history_lines(&state.dot_agtalk, "recv");
        assert!(recv_history
            .iter()
            .any(|h| h["type"] == "status" && h["to"] == "done" && h["reason"] == "msg.done"));
    }

    #[test]
    fn send_skips_receiver_history_when_no_session() {
        let (state, _tmp) = test_state();
        join(&state, "sender");
        let recv_addr =
            crate::identity::mailbox::create(&state.storage, "recv", "receiver", "").unwrap();

        let headers = auth_headers_for(&state, "sender");
        handle_send(
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

        let sender_history = read_history_lines(&state.dot_agtalk, "sender");
        assert_eq!(sender_history.len(), 1);
        assert!(!history_path(&state.dot_agtalk, "recv").exists());
    }

    fn history_path(dot: &std::path::Path, name: &str) -> std::path::PathBuf {
        dot.join(name).join("history.jsonl")
    }

    fn read_history_lines(dot: &std::path::Path, name: &str) -> Vec<serde_json::Value> {
        let path = history_path(dot, name);
        if !path.exists() {
            return Vec::new();
        }
        std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
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
