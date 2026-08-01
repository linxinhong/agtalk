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
                    // 飞书投递：卡片消息（未启用/无 feishu surface 时 no-op）
                    state
                        .feishu
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
pub(crate) fn participant_root(
    storage: &crate::storage::Storage,
    dot_agtalk: &Path,
    address: &str,
) -> Option<std::path::PathBuf> {
    let mb = match mailbox_db::get_by_address(storage, address) {
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

    if let Ok(session) = session_file::read(dot_agtalk, &mb.name) {
        if session.address == address {
            return Some(dot_agtalk.to_path_buf());
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
    let receiver_root = participant_root(&state.storage, &state.dot_agtalk, to_address);
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
    let original_root = participant_root(&state.storage, &state.dot_agtalk, &original.from_address);
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
    match participant_root(&state.storage, &state.dot_agtalk, &msg.to_address) {
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
    match participant_root(&state.storage, &state.dot_agtalk, &original.from_address) {
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
            // 飞书投递：卡片消息（未启用/无 feishu surface 时 no-op）
            state
                .feishu
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
#[path = "msg_tests.rs"]
mod tests;
