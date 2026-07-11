//! `/api/v1/human/*` handler：仅本机 human 客户端（popup/GUI）可用。
//!
//! 认证：`X-AgTalk-Human-Token`（identity::human_session，design §2.3 第三种受限例外），
//! 仅 human 域使用，agent / browser 凭据一律拒绝。所有操作以 human mailbox 地址为身份，
//! 复用 routing 与 human 领域模块；human surface 不允许直写 SQLite。

use crate::human::{self, approval, HumanError};
use crate::identity::human_session::{self, HumanSession};
use crate::identity::mailbox as mailbox_db;
use crate::notify;
use crate::proto::ServerMsg;
use crate::routing::{inbox, lookup, send, SendRequest};
use crate::server::handlers::{auth_error, id as id_handlers, msg as msg_handlers};
use crate::server::state::AppState;
use crate::transport::wake::SseEvent;
use axum::http::HeaderMap;

/// human token 认证 + 与 DB human 地址一致性校验（防 stale session 文件）。
#[allow(clippy::result_large_err)]
fn authenticate_human(state: &AppState, headers: &HeaderMap) -> Result<HumanSession, ServerMsg> {
    let token = headers
        .get("X-AgTalk-Human-Token")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| auth_error("缺少 X-AgTalk-Human-Token".into()))?;
    let session = human_session::validate(token).map_err(|e| auth_error(e.to_string()))?;
    let human_addr = human::human_address(&state.storage).map_err(|e| ServerMsg::Error {
        code: "human_mailbox_missing".into(),
        message: e.to_string(),
    })?;
    if session.address != human_addr {
        return Err(auth_error(
            "human session 与 mailbox 不一致，请重启 daemon 重新颁发".into(),
        ));
    }
    Ok(session)
}

fn err(code: &str, e: impl std::fmt::Display) -> ServerMsg {
    ServerMsg::Error {
        code: code.into(),
        message: e.to_string(),
    }
}

#[allow(clippy::result_large_err)]
fn resolve_human_id(state: &AppState, address: &str, id: &str) -> Result<String, ServerMsg> {
    lookup::resolve_id(&state.storage, address, id).map_err(|e| ServerMsg::Error {
        code: msg_handlers::routing_error_code(&e).into(),
        message: e.to_string(),
    })
}

fn human_error_msg(e: &HumanError) -> ServerMsg {
    let code = match e {
        HumanError::MessageNotFound(_) => "message_not_found",
        HumanError::AlreadyResolved { .. } => "already_resolved",
        HumanError::SelectOnlyRequiresChoice => "select_only_requires_choice",
        HumanError::InvalidChoice(_) => "invalid_choice",
        HumanError::DuplicateEvent { .. } => "duplicate_event",
        HumanError::AgentNotFound(_) => "agent_not_found",
        _ => "human_failed",
    };
    ServerMsg::Error {
        code: code.into(),
        message: e.to_string(),
    }
}

/// human 收到回复后发 SSE + notify 给原消息发送方；receiver 是本地 agent 时写 in history。
fn after_human_reply(state: &AppState, msg: &crate::routing::Message) {
    state.registry.notify(
        &msg.to_address,
        SseEvent {
            message: msg.clone(),
        },
    );
    let storage = state.storage.clone();
    let to = msg.to_address.clone();
    let message_id = msg.id.clone();
    let limiter = state.notify_limiter.clone();
    tokio::spawn(async move {
        if let Err(e) = notify::trigger(&storage, &to, "human", &message_id, &limiter, None).await {
            tracing::debug!("notify trigger skipped: {}", e);
        }
    });
    if let Some(root) = msg_handlers::participant_root(state, &msg.to_address) {
        if let Err(e) = crate::identity::history::append_message(
            &root,
            &msg.to_name,
            &msg.to_address,
            "in",
            msg,
            "human.reply",
        ) {
            tracing::warn!("receiver history append failed: {}", e);
        }
    }
}

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

pub fn handle_reply(
    state: &AppState,
    headers: &HeaderMap,
    message_id: String,
    body: String,
    choice: Option<String>,
    surface: String,
    external_event_id: Option<String>,
) -> ServerMsg {
    let session = match authenticate_human(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let resolved = match resolve_human_id(state, &session.address, &message_id) {
        Ok(r) => r,
        Err(e) => return e,
    };
    match approval::reply(
        &state.storage,
        approval::HumanReplyRequest {
            message_id: &resolved,
            body: &body,
            choice: choice.as_deref(),
            surface: &surface,
            external_event_id: external_event_id.as_deref(),
        },
    ) {
        Ok(out) => {
            after_human_reply(state, &out.reply);
            ServerMsg::Ok { id: out.reply.id }
        }
        Err(e) => human_error_msg(&e),
    }
}

pub fn handle_done(state: &AppState, headers: &HeaderMap, message_id: String) -> ServerMsg {
    let session = match authenticate_human(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let resolved = match resolve_human_id(state, &session.address, &message_id) {
        Ok(r) => r,
        Err(e) => return e,
    };
    match inbox::mark_done(&state.storage, &resolved, &session.address) {
        Ok(_) => ServerMsg::Ok { id: resolved },
        Err(e) => err("done_failed", e),
    }
}

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

/// human 主动发信：目标必须是活跃 mailbox，复用 routing::send（UUID 路由）。
pub fn handle_send(
    state: &AppState,
    headers: &HeaderMap,
    to: String,
    body: String,
    subject: Option<String>,
) -> ServerMsg {
    let session = match authenticate_human(state, headers) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let target = match mailbox_db::get_by_address(&state.storage, &to) {
        Ok(Some(mb)) if mb.left_at.is_none() => mb,
        Ok(_) => {
            return ServerMsg::Error {
                code: "agent_not_found".into(),
                message: format!("目标 agent 不存在或已离开: {}", to),
            }
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
    match send::send(&state.storage, req) {
        Ok(msg) => {
            after_human_reply(state, &msg);
            ServerMsg::Ok { id: msg.id }
        }
        Err(e) => err("send_failed", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgConfig;
    use crate::identity::mailbox::{create, ensure_human, mark_left};
    use crate::paths::CONFIG_DIR_ENV;
    use crate::routing::send::send;
    use std::ffi::OsString;
    use tempfile::TempDir;

    struct EnvGuard(Option<OsString>);

    impl EnvGuard {
        fn set(path: &std::path::Path) -> Self {
            let previous = std::env::var_os(CONFIG_DIR_ENV);
            std::env::set_var(CONFIG_DIR_ENV, path);
            Self(previous)
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(ref p) = self.0 {
                std::env::set_var(CONFIG_DIR_ENV, p);
            } else {
                std::env::remove_var(CONFIG_DIR_ENV);
            }
        }
    }

    struct Fixture {
        state: AppState,
        token: String,
        human_addr: String,
        _tmp: TempDir,
        _guard: EnvGuard,
    }

    fn setup() -> Fixture {
        let tmp = TempDir::new().unwrap();
        let guard = EnvGuard::set(tmp.path());
        let storage = crate::storage::Storage::open_in_memory().unwrap();
        let cfg = AgConfig::default();
        let human_addr = ensure_human(&storage, &cfg.human).unwrap();
        let session = human_session::ensure(&human_addr, &cfg.human.name).unwrap();
        let state = AppState::new(storage, cfg, tmp.path().join(".agtalk"));
        Fixture {
            state,
            token: session.token,
            human_addr,
            _tmp: tmp,
            _guard: guard,
        }
    }

    fn headers_with(token: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("X-AgTalk-Human-Token", token.parse().unwrap());
        h
    }

    fn send_to_human(fx: &Fixture, agent_addr: &str, content_type: &str, metadata: &str) -> String {
        send(
            &fx.state.storage,
            SendRequest {
                to: &fx.human_addr,
                to_name: "human",
                from: agent_addr,
                from_name: "agent",
                body: "question",
                content_type,
                reply_to_id: None,
                subject: None,
                metadata,
                more_coming: false,
            },
        )
        .unwrap()
        .id
    }

    #[test]
    fn token_auth_three_states() {
        let fx = setup();
        // 无 token
        match handle_inbox(&fx.state, &HeaderMap::new(), false) {
            ServerMsg::Error { code, .. } => assert_eq!(code, "auth_failed"),
            other => panic!("expected auth_failed, got {:?}", other),
        }
        // 错误 token
        match handle_inbox(&fx.state, &headers_with("wrong"), false) {
            ServerMsg::Error { code, .. } => assert_eq!(code, "auth_failed"),
            other => panic!("expected auth_failed, got {:?}", other),
        }
        // 正确 token
        match handle_inbox(&fx.state, &headers_with(&fx.token), false) {
            ServerMsg::InboxResult { messages } => assert!(messages.is_empty()),
            other => panic!("expected InboxResult, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn human_send_reaches_agent_inbox() {
        let fx = setup();
        let agent_addr = create(&fx.state.storage, "nora", "", "").unwrap();

        match handle_send(
            &fx.state,
            &headers_with(&fx.token),
            agent_addr.clone(),
            "hello nora".into(),
            Some("greet".into()),
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
            match handle_send(&fx.state, &headers_with(&fx.token), to, "hi".into(), None) {
                ServerMsg::Error { code, .. } => assert_eq!(code, "agent_not_found"),
                other => panic!("expected agent_not_found, got {:?}", other),
            }
        }
    }

    #[test]
    fn agents_list_excludes_human_and_left() {
        let fx = setup();
        create(&fx.state.storage, "active", "", "").unwrap();
        let left_addr = create(&fx.state.storage, "left", "", "").unwrap();
        mark_left(&fx.state.storage, &left_addr).unwrap();

        match handle_agents(&fx.state, &headers_with(&fx.token)) {
            ServerMsg::LookupResult { mailboxes } => {
                assert_eq!(mailboxes.len(), 1);
                assert_eq!(mailboxes[0].name, "active");
            }
            other => panic!("expected LookupResult, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn human_reply_routes_through_approval_arbitration() {
        let fx = setup();
        let agent_addr = create(&fx.state.storage, "asker", "", "").unwrap();
        let meta = r#"{"choices":["yes","no"],"select_only":true}"#;
        let msg_id = send_to_human(&fx, &agent_addr, "approval_request", meta);

        // select_only 无 choice → 拒绝
        match handle_reply(
            &fx.state,
            &headers_with(&fx.token),
            msg_id.clone(),
            "free text".into(),
            None,
            "popup".into(),
            None,
        ) {
            ServerMsg::Error { code, .. } => assert_eq!(code, "select_only_requires_choice"),
            other => panic!("expected select_only_requires_choice, got {:?}", other),
        }

        // 有效 choice → 胜出
        let first = handle_reply(
            &fx.state,
            &headers_with(&fx.token),
            msg_id[..8].to_string(),
            "ok".into(),
            Some("yes".into()),
            "popup".into(),
            None,
        );
        let reply_id = match first {
            ServerMsg::Ok { id } => id,
            other => panic!("expected Ok, got {:?}", other),
        };

        // 第二次审批 → already_resolved
        match handle_reply(
            &fx.state,
            &headers_with(&fx.token),
            msg_id,
            "no".into(),
            Some("no".into()),
            "gui".into(),
            None,
        ) {
            ServerMsg::Error { code, message } => {
                assert_eq!(code, "already_resolved");
                assert!(message.contains(&reply_id));
            }
            other => panic!("expected already_resolved, got {:?}", other),
        }

        // 回复落到 asker 的 inbox
        let agent_inbox = inbox::inbox(&fx.state.storage, &agent_addr, false).unwrap();
        assert!(agent_inbox.iter().any(|m| m.id == reply_id));
    }

    #[test]
    fn human_done_marks_message_done() {
        let fx = setup();
        let agent_addr = create(&fx.state.storage, "sender", "", "").unwrap();
        let msg_id = send_to_human(&fx, &agent_addr, "text", "{}");

        match handle_done(&fx.state, &headers_with(&fx.token), msg_id[..8].to_string()) {
            ServerMsg::Ok { id } => assert_eq!(id, msg_id),
            other => panic!("expected Ok, got {:?}", other),
        }
        let msg = lookup::detail(&fx.state.storage, &msg_id).unwrap().unwrap();
        assert_eq!(msg.status, "done");
    }

    #[test]
    fn human_read_marks_read_and_empty_inbox_stable() {
        let fx = setup();
        let agent_addr = create(&fx.state.storage, "sender", "", "").unwrap();
        send_to_human(&fx, &agent_addr, "text", "{}");

        match handle_read(&fx.state, &headers_with(&fx.token), None) {
            ServerMsg::InboxResult { messages } => assert_eq!(messages.len(), 1),
            other => panic!("expected InboxResult, got {:?}", other),
        }
        match handle_read(&fx.state, &headers_with(&fx.token), None) {
            ServerMsg::Error { code, .. } => assert_eq!(code, "inbox_empty"),
            other => panic!("expected inbox_empty, got {:?}", other),
        }
    }
}
