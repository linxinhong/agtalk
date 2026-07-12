//! `/api/v1/human/*` handler：仅本机 human 客户端（popup/GUI）可用。
//!
//! 认证：`X-AgTalk-Human-Token`（identity::human_session，design §2.3 第三种受限例外），
//! 仅 human 域使用，agent / browser 凭据一律拒绝。所有操作以 human mailbox 地址为身份，
//! 复用 routing 与 human 领域模块；human surface 不允许直写 SQLite。
//!
//! 按职责拆分：inbox（收件箱读取）/ actions（reply/done 审批动作）/ compose（发信与 agent 列表）。

mod actions;
mod compose;
mod inbox;

pub use actions::{handle_delivery_ack, handle_done, handle_reply};
pub use compose::{handle_agents, handle_send};
pub use inbox::{handle_inbox, handle_read};

use crate::human::{self, HumanError};
use crate::identity::human_session::{self, HumanSession};
use crate::notify;
use crate::proto::ServerMsg;
use crate::routing::lookup;
use crate::server::handlers::{auth_error, msg as msg_handlers};
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
        HumanError::AgentNotFound(_) => "agent_not_found",
        HumanError::ReceiptInconclusive { .. } => "receipt_inconclusive",
        HumanError::SingleChoiceOnly(_) => "single_choice_only",
        _ => "human_failed",
    };
    ServerMsg::Error {
        code: code.into(),
        message: e.to_string(),
    }
}

/// human 回复/发信后发 SSE + notify 给接收方；receiver 是本地 agent 时写 in history。
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
    if let Some(root) =
        msg_handlers::participant_root(&state.storage, &state.dot_agtalk, &msg.to_address)
    {
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

#[cfg(test)]
pub(crate) mod testkit {
    use super::*;
    use crate::config::AgConfig;
    use crate::identity::mailbox::ensure_human;
    use crate::paths::CONFIG_DIR_ENV;
    use crate::routing::{send::send, SendRequest};
    use std::ffi::OsString;
    use tempfile::TempDir;

    pub struct EnvGuard(Option<OsString>);

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

    pub struct Fixture {
        pub state: AppState,
        pub token: String,
        pub human_addr: String,
        _tmp: TempDir,
        _guard: EnvGuard,
    }

    pub fn setup() -> Fixture {
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

    pub fn headers_with(token: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("X-AgTalk-Human-Token", token.parse().unwrap());
        h
    }

    /// agent → human 发一条消息，返回消息 id。
    pub fn send_to_human(
        fx: &Fixture,
        agent_addr: &str,
        content_type: &str,
        metadata: &str,
    ) -> String {
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
}
