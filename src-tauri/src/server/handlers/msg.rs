//! `/api/v1/msg/*` handler。

use crate::identity::mailbox as mailbox_db;
use crate::identity::relations;
use crate::identity::session_file;
use crate::notify;
use crate::proto::{AskOptions, InboxFilter, ServerMsg};
use crate::routing::{inbox, lookup, reply, send, SendRequest};
use crate::server::handlers::{authenticate_req, not_supported};
use crate::server::state::AppState;
use crate::transport::wake::SseEvent;
use axum::http::HeaderMap;
use std::path::Path;

#[allow(clippy::too_many_arguments)]
pub fn handle_send(
    state: &AppState,
    headers: &HeaderMap,
    to: String,
    body: String,
    subject: Option<String>,
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
        subject: subject.as_deref(),
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
            record_send_relations(state, &session, &to_name, &to, &msg);
            fanout_if_human(state, &to, &msg);
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
        Ok((msg, original_status_change)) => {
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
            record_reply_history(
                state,
                &session,
                &original,
                &msg,
                original_status_change.as_ref(),
            );
            record_reply_relations(state, &session, &original, &msg);
            fanout_if_human(state, &msg.to_address, &msg);
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
                Ok(Some((m, _))) => m,
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
        Ok(Some(change)) => {
            record_status_history(
                state,
                &session,
                &target_msg,
                &change.old_status,
                &change.new_status,
                "msg.done",
            );
            ServerMsg::Ok { id: target_msg.id }
        }
        Ok(None) => ServerMsg::Ok { id: target_msg.id },
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
        subject: None,
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
            match crate::human::fanout(&state.storage, &state.config.human, &msg) {
                Ok(n) if n > 0 => {
                    // 桌面弹窗投递：审批消息拉起 agtalk __popup
                    state
                        .popup
                        .dispatch(&state.storage, &msg, &state.config.human.surfaces);
                }
                Ok(_) => {}
                Err(e) => tracing::warn!("human fanout failed: {}", e),
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
            Ok(Some((msg, Some(change)))) => {
                record_status_history(
                    state,
                    &session,
                    &msg,
                    &change.old_status,
                    &change.new_status,
                    "msg.read",
                );
                ServerMsg::MsgDetail(msg)
            }
            Ok(Some((msg, None))) => ServerMsg::MsgDetail(msg),
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
        // 读取真正未读（pending/delivered）消息并标记 read；空 inbox 返回稳定错误码 inbox_empty
        match inbox::unread_inbox(&state.storage, &session.address) {
            Ok(msgs) if msgs.is_empty() => ServerMsg::Error {
                code: "inbox_empty".into(),
                message: "当前 inbox 没有可查看的消息".into(),
            },
            Ok(msgs) => {
                for msg in &msgs {
                    if let Ok(Some(change)) = inbox::mark_read(&state.storage, &msg.id) {
                        record_status_history(
                            state,
                            &session,
                            msg,
                            &change.old_status,
                            &change.new_status,
                            "msg.read",
                        );
                    }
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

/// 解析某个参与者（非 sender）本地可达的 workspace 根（即 `.agtalk` 目录）。
///
/// 优先使用 mailbox 在 `id join` 时持久化的 `workspace_root`：仅当其为绝对路径、且
/// `<root>/<name>/session.json` 存在且 `session.address == address` 时采用；否则回退到
/// daemon 自身 workspace（`state.dot_agtalk`）下确实存在该 agent session 的情形，以兼容迁移
/// 前的存量单 workspace agent。都不可达时返回 `None`（远端 / human / 未在本机注册），
/// 调用方据此跳过该侧写入，不影响消息投递。
pub(crate) fn participant_root(state: &AppState, address: &str) -> Option<std::path::PathBuf> {
    let mb = match mailbox_db::get_by_address(&state.storage, address) {
        Ok(Some(mb)) => mb,
        _ => return None,
    };

    if !mb.workspace_root.is_empty() {
        let root = std::path::PathBuf::from(&mb.workspace_root);
        if root.is_absolute() {
            if let Ok(session) = session_file::read(&root, &mb.name) {
                if session.address == address {
                    return Some(root);
                }
            }
        }
    }

    if let Ok(session) = session_file::read(&state.dot_agtalk, &mb.name) {
        if session.address == address {
            return Some(state.dot_agtalk.clone());
        }
    }

    None
}

/// 获取 peer 的最新 intro：优先读取 peer 本地 session.json（确认 address 匹配），否则回退到 mailbox intro。
fn peer_intro(
    state: &AppState,
    peer_root: Option<&Path>,
    peer_address: &str,
    fallback: &str,
) -> String {
    let mb = match mailbox_db::get_by_address(&state.storage, peer_address) {
        Ok(Some(mb)) => mb,
        _ => return fallback.to_string(),
    };
    if let Some(root) = peer_root {
        if let Ok(session) = session_file::read(root, &mb.name) {
            if session.address == peer_address {
                return session.intro;
            }
        }
    }
    mb.intro
}

/// 发送成功后同时更新 sender / receiver 的 relations.json（仅当对应侧本地可达时）。
fn record_send_relations(
    state: &AppState,
    sender: &crate::identity::auth::AuthenticatedSession,
    to_name: &str,
    to_address: &str,
    msg: &crate::routing::Message,
) {
    let receiver_root = participant_root(state, to_address);
    let peer_intro_value = peer_intro(state, receiver_root.as_deref(), to_address, "");
    if let Err(e) = relations::record_send(
        &sender.workspace_root,
        relations::RelationEvent {
            owner_name: &sender.name,
            owner_address: &sender.address,
            peer_name: to_name,
            peer_address: to_address,
            peer_intro: &peer_intro_value,
            message_id: &msg.id,
            timestamp: msg.created_at,
        },
    ) {
        tracing::warn!("record sender relation failed: {}", e);
    }

    // receiver 也是本地可达 agent 时才写 receiver 侧关系。
    if let Some(root) = receiver_root {
        let sender_intro = peer_intro(state, Some(&sender.workspace_root), &sender.address, "");
        if let Err(e) = relations::record_receive(
            &root,
            relations::RelationEvent {
                owner_name: to_name,
                owner_address: to_address,
                peer_name: &sender.name,
                peer_address: &sender.address,
                peer_intro: &sender_intro,
                message_id: &msg.id,
                timestamp: msg.created_at,
            },
        ) {
            tracing::warn!("record receiver relation failed: {}", e);
        }
    }
}

/// 回复成功后同时更新 reply 发送方与原消息发送方的 relations.json。
fn record_reply_relations(
    state: &AppState,
    sender: &crate::identity::auth::AuthenticatedSession,
    original: &crate::routing::Message,
    reply: &crate::routing::Message,
) {
    let original_root = participant_root(state, &original.from_address);
    let peer_intro_value = peer_intro(state, original_root.as_deref(), &original.from_address, "");
    if let Err(e) = relations::record_send(
        &sender.workspace_root,
        relations::RelationEvent {
            owner_name: &sender.name,
            owner_address: &sender.address,
            peer_name: &original.from_name,
            peer_address: &original.from_address,
            peer_intro: &peer_intro_value,
            message_id: &reply.id,
            timestamp: reply.created_at,
        },
    ) {
        tracing::warn!("record reply sender relation failed: {}", e);
    }

    if let Some(root) = original_root {
        let sender_intro = peer_intro(state, Some(&sender.workspace_root), &sender.address, "");
        if let Err(e) = relations::record_receive(
            &root,
            relations::RelationEvent {
                owner_name: &original.from_name,
                owner_address: &original.from_address,
                peer_name: &sender.name,
                peer_address: &sender.address,
                peer_intro: &sender_intro,
                message_id: &reply.id,
                timestamp: reply.created_at,
            },
        ) {
            tracing::warn!("record reply receiver relation failed: {}", e);
        }
    }
}

fn record_send_history(
    state: &AppState,
    sender: &crate::identity::auth::AuthenticatedSession,
    to_name: &str,
    msg: &crate::routing::Message,
    source: &str,
) {
    if let Err(e) = crate::identity::history::append_message(
        &sender.workspace_root,
        &sender.name,
        &sender.address,
        "out",
        msg,
        source,
    ) {
        tracing::warn!("sender history append failed: {}", e);
    }
    match participant_root(state, &msg.to_address) {
        Some(root) => {
            if let Err(e) = crate::identity::history::append_message(
                &root,
                to_name,
                &msg.to_address,
                "in",
                msg,
                source,
            ) {
                tracing::warn!("receiver history append failed: {}", e);
            }
        }
        None => {
            tracing::debug!(
                to = %msg.to_address,
                to_name,
                "receiver workspace 不可达，跳过 receiver history"
            );
        }
    }
}

fn record_reply_history(
    state: &AppState,
    sender: &crate::identity::auth::AuthenticatedSession,
    original: &crate::routing::Message,
    reply: &crate::routing::Message,
    original_status_change: Option<&crate::routing::StatusChange>,
) {
    // reply 发送方的 out 事件
    if let Err(e) = crate::identity::history::append_message(
        &sender.workspace_root,
        &sender.name,
        &sender.address,
        "out",
        reply,
        "msg.reply",
    ) {
        tracing::warn!("reply sender history append failed: {}", e);
    }
    // 原消息发送方收到 reply 的 in 事件
    match participant_root(state, &original.from_address) {
        Some(root) => {
            if let Err(e) = crate::identity::history::append_message(
                &root,
                &original.from_name,
                &original.from_address,
                "in",
                reply,
                "msg.reply",
            ) {
                tracing::warn!("reply receiver history append failed: {}", e);
            }
        }
        None => {
            tracing::debug!(
                from = %original.from_address,
                "original sender workspace 不可达，跳过 reply in history"
            );
        }
    }
    // 原消息被 reply 后状态真实变化时才记录 status（归属操作者，即 reply 发送方）
    if let Some(change) = original_status_change {
        if let Err(e) = crate::identity::history::append_status(
            &sender.workspace_root,
            &sender.name,
            &sender.address,
            &change.message_id,
            &change.old_status,
            &change.new_status,
            "msg.reply",
        ) {
            tracing::warn!("reply status history append failed: {}", e);
        }
    }
}

fn record_status_history(
    _state: &AppState,
    owner: &crate::identity::auth::AuthenticatedSession,
    msg: &crate::routing::Message,
    old_status: &str,
    new_status: &str,
    reason: &str,
) {
    if old_status == new_status {
        return;
    }
    if let Err(e) = crate::identity::history::append_status(
        &owner.workspace_root,
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

/// 消息发往 human mailbox 时，按配置为各 surface 记账 pending delivery。
/// fanout 失败不影响消息投递（消息已落库 + SSE 已推送）。
fn fanout_if_human(state: &AppState, to_address: &str, msg: &crate::routing::Message) {
    let human_address = match crate::human::human_address(&state.storage) {
        Ok(addr) => addr,
        Err(e) => {
            tracing::debug!("human address lookup skipped: {}", e);
            return;
        }
    };
    if to_address != human_address {
        return;
    }
    match crate::human::fanout(&state.storage, &state.config.human, msg) {
        Ok(n) if n > 0 => {
            tracing::info!(message_id = %msg.id, surfaces = n, "human fanout recorded");
            // 桌面弹窗投递：为新消息拉起 agtalk __popup（未启用/无 popup surface 时 no-op）
            state
                .popup
                .dispatch(&state.storage, msg, &state.config.human.surfaces);
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!("human fanout failed: {}, running reconcile", e);
            // 消息已落库：reconcile 可按 to=human 全量补齐缺行，保证 delivery 可恢复
            match crate::human::reconcile(&state.storage, &state.config.human) {
                Ok(n) => tracing::info!(recovered = n, "human reconcile done"),
                Err(e2) => tracing::error!("human reconcile failed: {}", e2),
            }
        }
    }
}

pub(crate) fn routing_error_code(e: &crate::routing::RoutingError) -> &'static str {
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
    fn send_persists_trimmed_subject() {
        let (state, _tmp) = test_state();
        join(&state, "sender");
        let recv_addr = join(&state, "recv");

        let headers = auth_headers_for(&state, "sender");
        let resp = handle_send(
            &state,
            &headers,
            recv_addr,
            "body".into(),
            Some("  Review plan  ".into()),
            vec![],
            Some(false),
            None,
            false,
        );
        let id = match resp {
            ServerMsg::Ok { id } => id,
            other => panic!("expected Ok, got {:?}", other),
        };
        let msg = lookup::detail(&state.storage, &id).unwrap().unwrap();
        assert_eq!(msg.subject.as_deref(), Some("Review plan"));
    }

    #[test]
    fn send_blank_subject_becomes_none() {
        let (state, _tmp) = test_state();
        join(&state, "sender");
        let recv_addr = join(&state, "recv");

        let headers = auth_headers_for(&state, "sender");
        let resp = handle_send(
            &state,
            &headers,
            recv_addr,
            "body".into(),
            Some("   ".into()),
            vec![],
            Some(false),
            None,
            false,
        );
        let id = match resp {
            ServerMsg::Ok { id } => id,
            other => panic!("expected Ok, got {:?}", other),
        };
        let msg = lookup::detail(&state.storage, &id).unwrap().unwrap();
        assert!(msg.subject.is_none());
    }

    #[test]
    fn reply_inherits_subject() {
        let (state, _tmp) = test_state();
        join(&state, "sender");
        let recv_addr = join(&state, "recv");

        let sender_headers = auth_headers_for(&state, "sender");
        let send_resp = handle_send(
            &state,
            &sender_headers,
            recv_addr.clone(),
            "please review".into(),
            Some("Task X".into()),
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
        let reply_resp = handle_reply(
            &state,
            &recv_headers,
            sent_id[..8].to_string(),
            "done".into(),
            vec![],
            Some(false),
            None,
        );
        let reply_id = match reply_resp {
            ServerMsg::Ok { id } => id,
            other => panic!("expected Ok, got {:?}", other),
        };
        let reply_msg = lookup::detail(&state.storage, &reply_id).unwrap().unwrap();
        assert_eq!(reply_msg.subject.as_deref(), Some("Task X"));
    }

    #[test]
    fn send_writes_subject_to_history() {
        let (state, _tmp) = test_state();
        join(&state, "sender");
        let recv_addr = join(&state, "recv");

        let headers = auth_headers_for(&state, "sender");
        handle_send(
            &state,
            &headers,
            recv_addr,
            "body".into(),
            Some("History S".into()),
            vec![],
            Some(false),
            None,
            false,
        );

        let sender_history = read_history_lines(&state.dot_agtalk, "sender");
        let receiver_history = read_history_lines(&state.dot_agtalk, "recv");
        assert_eq!(sender_history[0]["subject"], "History S");
        assert_eq!(receiver_history[0]["subject"], "History S");
    }

    #[test]
    fn send_to_human_records_pending_deliveries() {
        let (state, _tmp) = test_state();
        join(&state, "sender");
        let human_addr =
            crate::identity::mailbox::ensure_human(&state.storage, &state.config.human).unwrap();

        let headers = auth_headers_for(&state, "sender");
        let resp = handle_send(
            &state,
            &headers,
            human_addr,
            "hi human".into(),
            None,
            vec![],
            Some(false),
            None,
            false,
        );
        let id = match resp {
            ServerMsg::Ok { id } => id,
            other => panic!("expected Ok, got {:?}", other),
        };

        let deliveries = crate::human::delivery::list_for_message(&state.storage, &id).unwrap();
        assert_eq!(deliveries.len(), state.config.human.surfaces.len());
        assert!(deliveries.iter().all(|d| d.status == "pending"));
    }

    #[test]
    fn send_to_agent_records_no_human_delivery() {
        let (state, _tmp) = test_state();
        join(&state, "sender");
        let recv_addr = join(&state, "recv");

        let headers = auth_headers_for(&state, "sender");
        let resp = handle_send(
            &state,
            &headers,
            recv_addr,
            "hi agent".into(),
            None,
            vec![],
            Some(false),
            None,
            false,
        );
        let id = match resp {
            ServerMsg::Ok { id } => id,
            other => panic!("expected Ok, got {:?}", other),
        };
        assert!(
            crate::human::delivery::list_for_message(&state.storage, &id)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn ask_records_pending_deliveries_for_human() {
        let (state, _tmp) = test_state();
        join(&state, "asker");

        let headers = auth_headers_for(&state, "asker");
        let resp = handle_ask(
            &state,
            &headers,
            "approve deploy?".into(),
            vec![],
            AskOptions {
                questions: vec![],
                options: vec!["yes".into(), "no".into()],
                recommended: Some("yes".into()),
                single: true,
                select_only: false,
            },
            false,
            None,
            false,
        );
        let id = match resp {
            ServerMsg::AskResult { message_id } => message_id,
            other => panic!("expected AskResult, got {:?}", other),
        };
        let deliveries = crate::human::delivery::list_for_message(&state.storage, &id).unwrap();
        assert_eq!(deliveries.len(), state.config.human.surfaces.len());
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

        // receiver: original in message (from send), out reply message, status for original pending->read (from handle_read)
        // 已 read 消息被 reply 不再产生 read->read 的冗余 status
        assert!(recv_history
            .iter()
            .any(|h| h["dir"] == "in" && h["source"] == "msg.send"));
        assert!(recv_history
            .iter()
            .any(|h| h["dir"] == "out" && h["source"] == "msg.reply"));
        assert!(recv_history
            .iter()
            .any(|h| h["type"] == "status" && h["reason"] == "msg.read"));
        assert!(!recv_history
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
    fn read_twice_does_not_duplicate_status() {
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
        let first = handle_read(&state, &recv_headers, None);
        assert!(matches!(first, ServerMsg::InboxResult { .. }));

        let second = handle_read(&state, &recv_headers, None);
        assert!(
            matches!(second, ServerMsg::Error { code, .. } if code == "inbox_empty"),
            "第二次 read 应返回 inbox_empty"
        );

        let recv_history = read_history_lines(&state.dot_agtalk, "recv");
        let status_events: Vec<_> = recv_history
            .iter()
            .filter(|h| h["type"] == "status")
            .collect();
        assert_eq!(status_events.len(), 1);
        assert_eq!(status_events[0]["from"], "pending");
        assert_eq!(status_events[0]["to"], "read");
        assert_eq!(status_events[0]["reason"], "msg.read");
    }

    #[test]
    fn read_done_message_does_not_write_status() {
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

        let done_history = read_history_lines(&state.dot_agtalk, "recv");
        let before_count = done_history
            .iter()
            .filter(|h| h["type"] == "status")
            .count();

        // 对已 done 消息执行 read 不应产生 done->read
        let read_resp = handle_read(&state, &recv_headers, None);
        assert!(
            matches!(read_resp, ServerMsg::Error { code, .. } if code == "inbox_empty"),
            "已 done 消息不应被 read 再次列出"
        );

        let recv_history = read_history_lines(&state.dot_agtalk, "recv");
        let after_count = recv_history
            .iter()
            .filter(|h| h["type"] == "status")
            .count();
        assert_eq!(before_count, after_count);
    }

    #[test]
    fn reply_read_message_does_not_write_read_to_read() {
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
        let after_read = read_history_lines(&state.dot_agtalk, "recv");
        let status_count_before = after_read.iter().filter(|h| h["type"] == "status").count();

        handle_reply(
            &state,
            &recv_headers,
            sent_id[..8].to_string(),
            "ok".into(),
            vec![],
            Some(false),
            None,
        );

        let after_reply = read_history_lines(&state.dot_agtalk, "recv");
        let status_count_after = after_reply.iter().filter(|h| h["type"] == "status").count();
        assert_eq!(
            status_count_before, status_count_after,
            "已 read 消息被 reply 不应再产生 status 事件"
        );
    }

    #[test]
    fn reply_done_message_does_not_write_done_to_read() {
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
        handle_done(
            &state,
            &recv_headers,
            Some(sent_id[..8].to_string()),
            None,
            vec![],
        );
        let after_done = read_history_lines(&state.dot_agtalk, "recv");
        let status_count_before = after_done.iter().filter(|h| h["type"] == "status").count();

        handle_reply(
            &state,
            &recv_headers,
            sent_id[..8].to_string(),
            "ok".into(),
            vec![],
            Some(false),
            None,
        );

        let after_reply = read_history_lines(&state.dot_agtalk, "recv");
        let status_count_after = after_reply.iter().filter(|h| h["type"] == "status").count();
        assert_eq!(
            status_count_before, status_count_after,
            "已 done 消息被 reply 不应再产生 status 事件"
        );
    }

    #[test]
    fn done_twice_does_not_duplicate_status() {
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
        let first = handle_done(&state, &recv_headers, None, None, vec![]);
        assert!(matches!(first, ServerMsg::Ok { .. }));

        let after_first = read_history_lines(&state.dot_agtalk, "recv");
        let status_count_before = after_first.iter().filter(|h| h["type"] == "status").count();

        let second = handle_done(&state, &recv_headers, None, None, vec![]);
        assert!(
            matches!(second, ServerMsg::Error { code, .. } if code == "inbox_empty"),
            "没有未完成消息时 done 应返回 inbox_empty"
        );

        let after_second = read_history_lines(&state.dot_agtalk, "recv");
        let status_count_after = after_second
            .iter()
            .filter(|h| h["type"] == "status")
            .count();
        assert_eq!(status_count_before, status_count_after);
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

    #[test]
    fn send_updates_relations_for_both_sides() {
        let (state, _tmp) = test_state();
        let sender_addr = join(&state, "sender");
        let recv_addr = join(&state, "recv");

        let headers = auth_headers_for(&state, "sender");
        handle_send(
            &state,
            &headers,
            recv_addr.clone(),
            "hi".into(),
            None,
            vec![],
            Some(false),
            None,
            false,
        );

        let sender_relations = crate::identity::relations::read(&state.dot_agtalk, "sender")
            .unwrap()
            .peers;
        let recv_relations = crate::identity::relations::read(&state.dot_agtalk, "recv")
            .unwrap()
            .peers;

        assert_eq!(sender_relations.len(), 1);
        let r = sender_relations
            .get(&recv_addr)
            .expect("sender should have recv peer");
        assert_eq!(r.name, "recv");
        assert_eq!(r.sent_count, 1);
        assert_eq!(r.received_count, 0);

        assert_eq!(recv_relations.len(), 1);
        let r = recv_relations
            .get(&sender_addr)
            .expect("recv should have sender peer");
        assert_eq!(r.name, "sender");
        assert_eq!(r.received_count, 1);
        assert_eq!(r.sent_count, 0);
    }

    #[test]
    fn send_skips_receiver_relation_when_no_session() {
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

        let sender_relations = crate::identity::relations::read(&state.dot_agtalk, "sender")
            .unwrap()
            .peers;
        assert_eq!(sender_relations.len(), 1);

        assert!(!state
            .dot_agtalk
            .join("recv")
            .join("relations.json")
            .exists());
    }

    #[test]
    fn reply_updates_relations_for_both_sides() {
        let (state, _tmp) = test_state();
        let sender_addr = join(&state, "sender");
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
        let reply_resp = handle_reply(
            &state,
            &recv_headers,
            sent_id[..8].to_string(),
            "ok".into(),
            vec![],
            Some(false),
            None,
        );
        assert!(
            matches!(reply_resp, ServerMsg::Ok { .. }),
            "reply should succeed, got {:?}",
            reply_resp
        );

        let sender_relations = crate::identity::relations::read(&state.dot_agtalk, "sender")
            .unwrap()
            .peers;
        let recv_relations = crate::identity::relations::read(&state.dot_agtalk, "recv")
            .unwrap()
            .peers;

        // sender: original out + reply in
        let r = sender_relations
            .get(&recv_addr)
            .expect("sender should have recv peer");
        assert_eq!(r.received_count, 1);
        assert_eq!(r.sent_count, 1);

        // recv: original in + reply out
        let r = recv_relations
            .get(&sender_addr)
            .expect("recv should have sender peer");
        assert_eq!(r.sent_count, 1);
        assert_eq!(r.received_count, 1);
    }

    // ---- 跨 workspace：history/relations 写入各 agent 自身根 ----

    fn test_state_at(daemon_dot: &std::path::Path) -> AppState {
        let storage = Storage::open_in_memory().unwrap();
        AppState::new(storage, AgConfig::default(), daemon_dot.to_path_buf())
    }

    fn join_at(state: &AppState, root: &std::path::Path, name: &str) -> String {
        let pid = std::process::id();
        let start_time = current_pid_start_time();
        match id::handle_join(
            state,
            root,
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

    fn auth_headers_for_root(root: &std::path::Path, name: &str) -> HeaderMap {
        let session = crate::identity::session_file::read(root, name).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("X-AgTalk-Address", session.address.parse().unwrap());
        headers.insert(
            "X-AgTalk-Workspace-Root",
            root.to_str().unwrap().parse().unwrap(),
        );
        headers
    }

    #[test]
    fn cross_workspace_history_written_to_each_agent_root() {
        let daemon_tmp = TempDir::new().unwrap();
        let sender_tmp = TempDir::new().unwrap();
        let recv_tmp = TempDir::new().unwrap();
        let daemon_dot = daemon_tmp.path().join(".agtalk");
        let sender_dot = sender_tmp.path().join(".agtalk");
        let recv_dot = recv_tmp.path().join(".agtalk");

        let state = test_state_at(&daemon_dot);
        join_at(&state, &sender_dot, "sender");
        let recv_addr = join_at(&state, &recv_dot, "recv");

        let headers = auth_headers_for_root(&sender_dot, "sender");
        let resp = handle_send(
            &state,
            &headers,
            recv_addr,
            "hi cross".into(),
            None,
            vec![],
            Some(false),
            None,
            false,
        );
        assert!(matches!(resp, ServerMsg::Ok { .. }), "send ok: {:?}", resp);

        let sender_history = read_history_lines(&sender_dot, "sender");
        assert_eq!(sender_history.len(), 1);
        assert_eq!(sender_history[0]["dir"], "out");
        assert_eq!(sender_history[0]["source"], "msg.send");

        let recv_history = read_history_lines(&recv_dot, "recv");
        assert_eq!(recv_history.len(), 1);
        assert_eq!(recv_history[0]["dir"], "in");

        // daemon 根目录下不应生成任何 agent 目录。
        assert!(!daemon_dot.join("sender").exists());
        assert!(!daemon_dot.join("recv").exists());
    }

    #[test]
    fn cross_workspace_reply_history_and_relations() {
        let daemon_tmp = TempDir::new().unwrap();
        let sender_tmp = TempDir::new().unwrap();
        let recv_tmp = TempDir::new().unwrap();
        let daemon_dot = daemon_tmp.path().join(".agtalk");
        let sender_dot = sender_tmp.path().join(".agtalk");
        let recv_dot = recv_tmp.path().join(".agtalk");

        let state = test_state_at(&daemon_dot);
        let sender_addr = join_at(&state, &sender_dot, "sender");
        let recv_addr = join_at(&state, &recv_dot, "recv");

        let sender_headers = auth_headers_for_root(&sender_dot, "sender");
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

        let recv_headers = auth_headers_for_root(&recv_dot, "recv");
        let reply_resp = handle_reply(
            &state,
            &recv_headers,
            sent_id[..8].to_string(),
            "ok".into(),
            vec![],
            Some(false),
            None,
        );
        assert!(
            matches!(reply_resp, ServerMsg::Ok { .. }),
            "reply ok: {:?}",
            reply_resp
        );

        // sender 根：send out + reply in
        let sender_history = read_history_lines(&sender_dot, "sender");
        assert!(
            sender_history.iter().any(|e| e["dir"] == "in"),
            "sender 根应记录 reply 的 in 事件"
        );
        // recv 根：send in + reply out
        let recv_history = read_history_lines(&recv_dot, "recv");
        assert!(
            recv_history.iter().any(|e| e["dir"] == "out"),
            "recv 根应记录 reply 的 out 事件"
        );

        // relations 也写入各自根
        let sender_relations = crate::identity::relations::read(&sender_dot, "sender")
            .unwrap()
            .peers;
        let recv_relations = crate::identity::relations::read(&recv_dot, "recv")
            .unwrap()
            .peers;
        assert_eq!(
            sender_relations
                .get(&recv_addr)
                .expect("sender has recv peer")
                .received_count,
            1
        );
        assert_eq!(
            recv_relations
                .get(&sender_addr)
                .expect("recv has sender peer")
                .sent_count,
            1
        );

        // daemon 根不下错写任何 agent 目录
        assert!(!daemon_dot.join("sender").exists());
        assert!(!daemon_dot.join("recv").exists());
    }

    #[test]
    fn send_to_receiver_without_local_session_skips_receiver_side() {
        let daemon_tmp = TempDir::new().unwrap();
        let sender_tmp = TempDir::new().unwrap();
        let daemon_dot = daemon_tmp.path().join(".agtalk");
        let sender_dot = sender_tmp.path().join(".agtalk");

        let state = test_state_at(&daemon_dot);
        join_at(&state, &sender_dot, "sender");
        // 仅在 DB 建 mailbox（workspace_root=""），不创建任何 session 文件。
        let recv_addr =
            crate::identity::mailbox::create(&state.storage, "recv", "receiver", "").unwrap();

        let headers = auth_headers_for_root(&sender_dot, "sender");
        let resp = handle_send(
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
        assert!(
            matches!(resp, ServerMsg::Ok { .. }),
            "send to non-local receiver should still succeed: {:?}",
            resp
        );

        // sender 侧写入
        let sender_history = read_history_lines(&sender_dot, "sender");
        assert_eq!(sender_history.len(), 1);
        assert_eq!(sender_history[0]["dir"], "out");

        // receiver 侧不可达：daemon 根下也不应错写 recv 目录
        assert!(!daemon_dot.join("recv").exists());
        assert!(!daemon_dot.join("sender").exists());
    }

    #[test]
    fn done_status_written_to_actor_root_in_cross_workspace() {
        let daemon_tmp = TempDir::new().unwrap();
        let sender_tmp = TempDir::new().unwrap();
        let recv_tmp = TempDir::new().unwrap();
        let daemon_dot = daemon_tmp.path().join(".agtalk");
        let sender_dot = sender_tmp.path().join(".agtalk");
        let recv_dot = recv_tmp.path().join(".agtalk");

        let state = test_state_at(&daemon_dot);
        join_at(&state, &sender_dot, "sender");
        let recv_addr = join_at(&state, &recv_dot, "recv");

        let sender_headers = auth_headers_for_root(&sender_dot, "sender");
        let send_resp = handle_send(
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
        let sent_id = match send_resp {
            ServerMsg::Ok { id } => id,
            other => panic!("expected Ok, got {:?}", other),
        };

        // recv 标记完成：status 事件应写入 recv（操作者）的根。
        let recv_headers = auth_headers_for_root(&recv_dot, "recv");
        let done_resp = handle_done(
            &state,
            &recv_headers,
            Some(sent_id[..8].to_string()),
            None,
            vec![],
        );
        assert!(
            matches!(done_resp, ServerMsg::Ok { .. }),
            "done ok: {:?}",
            done_resp
        );

        let recv_history = read_history_lines(&recv_dot, "recv");
        assert!(
            recv_history
                .iter()
                .any(|e| e["type"] == "status" && e["reason"] == "msg.done"),
            "recv 根应记录 msg.done 的 status 事件"
        );

        // sender 根不应出现 status 事件（只有 send 的 out）
        let sender_history = read_history_lines(&sender_dot, "sender");
        assert!(
            sender_history.iter().all(|e| e["type"] != "status"),
            "sender 根不应写入 done 的 status 事件"
        );
    }
}
